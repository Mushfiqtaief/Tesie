use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use bytes::Bytes;
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tesie_core::{Result, TesRouteError, WireFormat};

use crate::opencode::session::upstream_now_ms;

/// Maps upstream wire tool names (`mcp_<12hex>` or bound official aliases) back to
/// the downstream caller's original tool names.
pub type ToolRenameMap = HashMap<String, String>;

const OPENCODE_PROMPT_V2: &str = include_str!("opencode_prompt_v2.txt");
const OPENCODE_TOOLS_V2_JSON: &str = include_str!("opencode_tools.json");

const OPENCODE_PROMPT_MARKER: &str = "You are opencode, an interactive CLI tool";
const OPENCODE_SKILLS_BLOCK: &str = r#"Skills provide specialized instructions and workflows for specific tasks.
Use the skill tool to load a skill when a task matches its description.
<available_skills>
  <skill>
    <name>customize-opencode</name>
    <description>Use ONLY when the user is editing or creating opencode's own configuration: opencode.json, opencode.jsonc, files under .opencode/, or files under ~/.config/opencode/. Also use when creating or fixing opencode agents, subagents, skills, plugins, MCP servers, or permission rules. Do not use for the user's own application code, or for any project that is not configuring opencode itself.</description>
    <location>&lt;built-in&gt;</location>
  </skill>
</available_skills>"#;

const CLIENT_TOOL_PREFIX: &str = "mcp_";
const CLIENT_TOOL_DIGEST_LEN: usize = 12;
const MAX_TOOL_NAME_LEN: usize = 64;
const V2_COMPLETION_LIMIT: i64 = 32_000;

const V2_OFFICIAL_TOOL_ORDER: [&str; 13] = [
    "edit",
    "execute",
    "glob",
    "grep",
    "question",
    "read",
    "shell",
    "skill",
    "subagent",
    "webfetch",
    "websearch",
    "write",
    "patch",
];

const V2_CHAT_KEY_ORDER: [&str; 6] = [
    "model",
    "messages",
    "tools",
    "stream",
    "stream_options",
    "max_completion_tokens",
];

#[derive(Debug, Clone, Deserialize)]
struct OfficialToolDef {
    description: String,
    parameters: Value,
}

static OFFICIAL_TOOLS_V2: LazyLock<HashMap<String, OfficialToolDef>> = LazyLock::new(|| {
    serde_json::from_str::<HashMap<String, OfficialToolDef>>(OPENCODE_TOOLS_V2_JSON)
        .unwrap_or_default()
});

static WORKSPACE_DIR: LazyLock<String> = LazyLock::new(|| {
    std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .filter(|s| !s.trim().is_empty() && s != ".")
        .unwrap_or_else(|| "/workspace".to_string())
});

fn host_platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

fn v2_shell_description(base: &str, platform: &str) -> String {
    match platform {
        "win32" | "windows" => {
            base.replacen("on Linux using bash", "on Windows using powershell", 1)
        }
        "darwin" => base.replacen("on Linux using bash", "on macOS using bash", 1),
        _ => base.to_string(),
    }
}

fn render_opencode_environment(model_id: &str) -> String {
    let api_model = model_id
        .trim()
        .strip_prefix("opencode/")
        .or_else(|| model_id.trim().strip_prefix("oc/"))
        .unwrap_or(model_id.trim());
    let clean_model = if api_model.is_empty() {
        "unknown"
    } else {
        api_model
    };
    let dir = WORKSPACE_DIR.as_str();
    let platform = host_platform();
    let now_ts = upstream_now_ms();
    let date_str = Utc
        .timestamp_millis_opt(now_ts)
        .earliest()
        .unwrap_or_else(Utc::now)
        .format("%a %b %d %Y")
        .to_string();

    [
        format!("You are powered by the model named {clean_model}. The exact model ID is opencode/{clean_model}"),
        "Here is some useful information about the environment you are running in:".to_string(),
        "<env>".to_string(),
        format!("  Working directory: {dir}"),
        format!("  Workspace root folder: {dir}"),
        "  Is directory a git repo: yes".to_string(),
        format!("  Platform: {platform}"),
        format!("  Today's date: {date_str}"),
        "</env>".to_string(),
    ]
    .join("\n")
}

fn build_official_v2_system_prompt(model_id: &str) -> String {
    format!(
        "{OPENCODE_PROMPT_V2}{}\n{OPENCODE_SKILLS_BLOCK}",
        render_opencode_environment(model_id)
    )
}

fn value_contains_prompt_marker(val: &Value) -> bool {
    if let Some(s) = val.as_str() {
        return s.contains(OPENCODE_PROMPT_MARKER);
    }
    if let Some(arr) = val.as_array() {
        return arr.iter().any(|b| {
            b.get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| t.contains(OPENCODE_PROMPT_MARKER))
        });
    }
    false
}

fn extract_text_from_content(val: &Value) -> String {
    if let Some(s) = val.as_str() {
        return s.trim().to_string();
    }
    if let Some(arr) = val.as_array() {
        let parts: Vec<&str> = arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str).map(str::trim))
            .filter(|s| !s.is_empty())
            .collect();
        return parts.join("\n\n");
    }
    String::new()
}

const TITLE_PROMPTS_V2: [&str; 7] = [
    "you name conversations so the user can find them later in a list of past sessions",
    "you generate a descriptive title for this conversation",
    "you decide whether the session title still matches the conversation",
    "reply with a title and nothing else",
    "reply with a title for it and nothing else",
    "generate a brief title",
    "short title for this conversation",
];
const TITLE_PROMPT_V1: &str = "generate a title for this conversation";

fn is_title_generation_request(obj: &Map<String, Value>) -> bool {
    if obj
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return false;
    }
    if obj
        .get("functions")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return false;
    }
    let Some(messages) = obj.get("messages").and_then(Value::as_array) else {
        return false;
    };
    if messages.is_empty() || messages.len() > 3 {
        return false;
    }

    let mut matched = false;
    for msg in messages {
        let Some(mobj) = msg.as_object() else {
            continue;
        };
        let role = mobj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(role.as_str(), "assistant" | "tool" | "function") {
            return false;
        }
        let Some(content) = mobj.get("content") else {
            continue;
        };
        let lower = extract_text_from_content(content).to_ascii_lowercase();
        if lower.is_empty() {
            continue;
        }
        match role.as_str() {
            "system" | "developer" => {
                if TITLE_PROMPTS_V2.iter().any(|p| lower.contains(p)) {
                    matched = true;
                }
            }
            "user" => {
                if lower.starts_with(TITLE_PROMPT_V1) {
                    matched = true;
                }
            }
            _ => {}
        }
    }
    matched
}

pub fn is_opencode_title_request(body: &Value) -> bool {
    body.as_object()
        .is_some_and(is_title_generation_request)
}

pub fn synthesize_title_from_body(body: &Value) -> String {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return "New Session".to_string();
    };
    let mut user_text = String::new();
    for msg in messages {
        if msg.get("role").and_then(Value::as_str) == Some("user")
            && let Some(content) = msg.get("content")
        {
            let t = extract_text_from_content(content);
            if !t.is_empty() {
                user_text = t;
                break;
            }
        }
    }
    if user_text.to_ascii_lowercase().starts_with(TITLE_PROMPT_V1)
        && let Some((_, rest)) = user_text.split_once('\n')
    {
        user_text = rest.trim().to_string();
    }
    let first_line = user_text.lines().next().unwrap_or("").trim();
    let words: Vec<&str> = first_line.split_whitespace().take(6).collect();
    let joined: String = words.join(" ").chars().take(40).collect();
    let clean = joined.trim_matches(|c: char| matches!(c, '"' | '\'' | ' ' | '\t' | '.'));
    if clean.is_empty() {
        "New Session".to_string()
    } else {
        clean.to_string()
    }
}

/// Builds a minimal OpenCode 2.x v2 agent-shaped request body for background
/// 30-second per-IP quota checks (matching `opencode2api/internal/gateway/probe.go`
/// `probePayload` byte-for-byte): `"Reply with OK."`, `max_tokens: 64`, `stream: true`,
/// official v2 CLI prompt + `<env>` + `<available_skills>`, and the 13 official v2 tools.
/// Upstream requires the 13-tool agent shape on anonymous `/inference` free models and
/// returns synthetic `429 FreeUsageLimitError` when tools are omitted; dropping the SSE
/// stream as soon as `200 OK` headers arrive keeps completion usage near zero.
pub fn build_minimal_quota_probe_body(model_id: &str) -> Result<Bytes> {
    let mut body = json!({
        "model": model_id,
        "stream": true,
        "max_tokens": 64,
        "messages": [
            {
                "role": "user",
                "content": "Reply with OK."
            }
        ]
    });
    normalize_opencode_request_for_lane(model_id, &mut body, WireFormat::OpenAiChat);
    apply_fingerprint_tools(&mut body, WireFormat::OpenAiChat);
    encode_opencode_body(&body, WireFormat::OpenAiChat)
}

/// Prepends the genuine OpenCode 2.x CLI system prompt + `<env>` + `<available_skills>`
/// ahead of any caller system/developer instructions, matching `opencode2api`'s
/// `injectOfficialPrompt` across Chat, Responses, and Anthropic Messages.
pub fn inject_official_v2_prompt(
    model_id: &str,
    body: &mut Map<String, Value>,
    target_format: WireFormat,
) {
    let official = build_official_v2_system_prompt(model_id);

    match target_format {
        WireFormat::OpenAiChat => {
            let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
                return;
            };
            if messages.iter().any(|m| {
                matches!(
                    m.get("role").and_then(Value::as_str),
                    Some("system" | "developer")
                ) && m.get("content").is_some_and(value_contains_prompt_marker)
            }) {
                return;
            }

            let mut caller_sys_parts = Vec::new();
            let mut non_sys_messages = Vec::with_capacity(messages.len() + 1);
            for msg in messages.drain(..) {
                let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
                if role == "system" || role == "developer" {
                    if let Some(c) = msg.get("content") {
                        let extracted = extract_text_from_content(c);
                        if !extracted.is_empty() {
                            caller_sys_parts.push(extracted);
                        }
                    }
                } else {
                    non_sys_messages.push(msg);
                }
            }

            let merged = if caller_sys_parts.is_empty() {
                official
            } else {
                format!("{official}\n\n{}", caller_sys_parts.join("\n\n"))
            };
            non_sys_messages.insert(0, json!({ "role": "system", "content": merged }));
            *messages = non_sys_messages;
        }
        WireFormat::OpenAiResponses => {
            if let Some(instr) = body.get("instructions")
                && value_contains_prompt_marker(instr)
            {
                return;
            }
            let existing = body
                .get("instructions")
                .map(extract_text_from_content)
                .unwrap_or_default();
            let merged = if existing.is_empty() {
                official
            } else {
                format!("{official}\n\n{existing}")
            };
            body.insert("instructions".into(), Value::String(merged));
        }
        WireFormat::AnthropicMessages => {
            if let Some(sys) = body.get("system")
                && value_contains_prompt_marker(sys)
            {
                return;
            }
            let mut blocks = vec![json!({ "type": "text", "text": official })];
            match body.get("system") {
                Some(Value::String(s)) if !s.trim().is_empty() => {
                    blocks.push(json!({ "type": "text", "text": s.trim() }));
                }
                Some(Value::Array(arr)) => {
                    for b in arr {
                        blocks.push(b.clone());
                    }
                }
                _ => {}
            }
            body.insert("system".into(), Value::Array(blocks));
        }
    }
}

fn client_tool_wire_name(client_name: &str, used: &mut HashSet<String>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(client_name.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let short = &digest[..CLIENT_TOOL_DIGEST_LEN];
    let base = format!("{CLIENT_TOOL_PREFIX}{short}");
    if used.insert(base.clone()) {
        return base;
    }
    let mut suffix = 2usize;
    loop {
        let tag = format!("_{suffix}");
        let cut = MAX_TOOL_NAME_LEN.saturating_sub(tag.len()).min(base.len());
        let candidate = format!("{}{tag}", &base[..cut]);
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn tool_name_of(tool: &Value) -> Option<String> {
    let obj = tool.as_object()?;
    if let Some(name) = obj.get("name").and_then(Value::as_str) {
        let t = name.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Some(fn_obj) = obj.get("function").and_then(Value::as_object)
        && let Some(name) = fn_obj.get("name").and_then(Value::as_str)
    {
        let t = name.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    None
}

fn set_tool_wire_name(mut tool: Value, target_format: WireFormat, wire_name: &str) -> Value {
    if let Some(obj) = tool.as_object_mut() {
        match target_format {
            WireFormat::OpenAiChat => {
                if let Some(fn_obj) = obj.get_mut("function").and_then(Value::as_object_mut) {
                    fn_obj.insert("name".into(), Value::String(wire_name.to_string()));
                    if !fn_obj.contains_key("strict") {
                        fn_obj.insert("strict".into(), Value::Bool(false));
                    }
                } else {
                    let desc = obj
                        .remove("description")
                        .unwrap_or_else(|| Value::String(String::new()));
                    let params = obj
                        .remove("parameters")
                        .or_else(|| obj.remove("input_schema"))
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                    obj.clear();
                    obj.insert("type".into(), Value::String("function".into()));
                    obj.insert(
                        "function".into(),
                        json!({
                            "name": wire_name,
                            "description": desc,
                            "parameters": params,
                            "strict": false
                        }),
                    );
                }
            }
            WireFormat::AnthropicMessages => {
                if let Some(fn_obj) = obj.remove("function").and_then(|v| v.as_object().cloned()) {
                    let desc = fn_obj
                        .get("description")
                        .cloned()
                        .unwrap_or_else(|| Value::String(String::new()));
                    let schema = fn_obj
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                    obj.clear();
                    obj.insert("name".into(), Value::String(wire_name.to_string()));
                    obj.insert("description".into(), desc);
                    obj.insert("input_schema".into(), schema);
                } else {
                    obj.insert("name".into(), Value::String(wire_name.to_string()));
                }
            }
            WireFormat::OpenAiResponses => {
                obj.insert("name".into(), Value::String(wire_name.to_string()));
            }
        }
    }
    tool
}

fn build_official_v2_tool(name: &str, target_format: WireFormat, platform: &str) -> Value {
    let (mut description, parameters) = match OFFICIAL_TOOLS_V2.get(name) {
        Some(def) => (def.description.clone(), def.parameters.clone()),
        None => (
            format!("Agent tool {name}"),
            json!({ "type": "object", "properties": {} }),
        ),
    };
    if name == "shell" {
        description = v2_shell_description(&description, platform);
    }

    match target_format {
        WireFormat::OpenAiChat => json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
                "strict": false
            }
        }),
        WireFormat::AnthropicMessages => json!({
            "name": name,
            "description": description,
            "input_schema": parameters
        }),
        WireFormat::OpenAiResponses => json!({
            "type": "function",
            "name": name,
            "description": description,
            "parameters": parameters
        }),
    }
}

fn official_tool_candidates(official_name: &str) -> &'static [&'static str] {
    match official_name {
        "shell" => &["Bash", "bash", "shell"],
        "edit" => &["Edit", "edit"],
        "glob" => &["Glob", "glob"],
        "grep" => &["Grep", "grep"],
        "question" => &["AskUserQuestion", "ask_question", "question"],
        "read" => &["Read", "read", "read_file"],
        "skill" => &["Skill", "skill"],
        "subagent" => &["Agent", "Task", "Subagent", "subagent"],
        "webfetch" => &["WebFetch", "webfetch"],
        "websearch" => &["WebSearch", "websearch", "search_web"],
        "write" => &["Write", "write", "write_file"],
        "patch" => &["Patch", "patch", "apply_patch"],
        _ => &[],
    }
}

fn fold_tool_name(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '_' | '.' | '-' | ' ' | '\t' | '\r' | '\n'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn retarget_tool_choice(body: &mut Map<String, Value>, client_to_wire: &HashMap<String, String>) {
    if client_to_wire.is_empty() {
        return;
    }
    let Some(choice) = body.get_mut("tool_choice").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(name) = choice.get("name").and_then(Value::as_str)
        && let Some(wire) = client_to_wire.get(name.trim())
    {
        choice.insert("name".into(), Value::String(wire.clone()));
        return;
    }
    if let Some(fn_obj) = choice.get_mut("function").and_then(Value::as_object_mut)
        && let Some(name) = fn_obj.get("name").and_then(Value::as_str)
        && let Some(wire) = client_to_wire.get(name.trim())
    {
        fn_obj.insert("name".into(), Value::String(wire.clone()));
    }
}

fn alias_history_tool_calls(
    body: &mut Map<String, Value>,
    client_to_wire: &HashMap<String, String>,
) {
    if client_to_wire.is_empty() {
        return;
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if let Some(obj) = item.as_object_mut()
                && obj.get("type").and_then(Value::as_str) == Some("function_call")
                && let Some(name) = obj.get("name").and_then(Value::as_str)
                && let Some(wire) = client_to_wire.get(name)
            {
                obj.insert("name".into(), Value::String(wire.clone()));
            }
        }
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for msg in messages {
            let Some(msg_obj) = msg.as_object_mut() else {
                continue;
            };
            if let Some(tool_calls) = msg_obj.get_mut("tool_calls").and_then(Value::as_array_mut) {
                for call in tool_calls {
                    if let Some(fn_obj) = call
                        .as_object_mut()
                        .and_then(|c| c.get_mut("function"))
                        .and_then(Value::as_object_mut)
                        && let Some(name) = fn_obj.get("name").and_then(Value::as_str)
                        && let Some(wire) = client_to_wire.get(name)
                    {
                        fn_obj.insert("name".into(), Value::String(wire.clone()));
                    }
                }
            }
            if let Some(blocks) = msg_obj.get_mut("content").and_then(Value::as_array_mut) {
                for block in blocks {
                    if let Some(bobj) = block.as_object_mut()
                        && bobj.get("type").and_then(Value::as_str) == Some("tool_use")
                        && let Some(name) = bobj.get("name").and_then(Value::as_str)
                        && let Some(wire) = client_to_wire.get(name)
                    {
                        bobj.insert("name".into(), Value::String(wire.clone()));
                    }
                }
            }
        }
    }
}

/// Renames every client tool to an opaque `mcp_<12hex>` wire identifier, appends the
/// full 13-tool official OpenCode 2.x CLI toolset in exact order, injects the official
/// OpenCode CLI system prompt + `<env>` + `<available_skills>`, and returns the inverse
/// map (`wire_name -> client_name`) so streaming/non-streaming responses restore caller names.
pub fn apply_fingerprint_tools(body: &mut Value, target_format: WireFormat) -> ToolRenameMap {
    let Some(obj) = body.as_object_mut() else {
        return ToolRenameMap::new();
    };
    let model_id = obj
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    inject_official_v2_prompt(&model_id, obj, target_format);

    if is_title_generation_request(obj) {
        return ToolRenameMap::new();
    }

    let mut incoming_tools = obj
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Support legacy OpenAI `functions[]` array
    if matches!(
        target_format,
        WireFormat::OpenAiChat | WireFormat::OpenAiResponses
    ) && let Some(legacy_fns) = obj.remove("functions").and_then(|v| v.as_array().cloned())
    {
        for f in legacy_fns {
            incoming_tools.push(json!({ "type": "function", "function": f }));
        }
    }

    let official_set: HashSet<&str> = V2_OFFICIAL_TOOL_ORDER.iter().copied().collect();
    let mut used_wire_names: HashSet<String> = V2_OFFICIAL_TOOL_ORDER
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let mut wire_to_client = ToolRenameMap::new();
    let mut client_to_wire: HashMap<String, String> = HashMap::new();
    let mut declared_client_names: Vec<String> = Vec::new();
    let mut shaped_tools: Vec<Value> =
        Vec::with_capacity(incoming_tools.len() + V2_OFFICIAL_TOOL_ORDER.len());

    let platform = host_platform();

    for tool in incoming_tools {
        let Some(client_name) = tool_name_of(&tool) else {
            shaped_tools.push(tool);
            continue;
        };

        // If the client already sent an exact official OpenCode tool (`read`, `shell`, etc.)
        // without conflicting with our appended official set, let the official definition win
        // unless its schema differs — renaming every client-declared tool to `mcp_<12hex>` keeps
        // the client's exact JSON schema intact while appending the 13 official tools verbatim!
        let wire_name = if let Some(existing) = client_to_wire.get(&client_name) {
            existing.clone()
        } else {
            let minted = client_tool_wire_name(&client_name, &mut used_wire_names);
            client_to_wire.insert(client_name.clone(), minted.clone());
            wire_to_client.insert(minted.clone(), client_name.clone());
            declared_client_names.push(client_name.clone());
            minted
        };

        shaped_tools.push(set_tool_wire_name(tool, target_format, &wire_name));
    }

    // Append all 13 official OpenCode 2.x CLI tools in exact captured order
    for official_name in V2_OFFICIAL_TOOL_ORDER {
        shaped_tools.push(build_official_v2_tool(official_name, target_format, platform));
    }

    // Bind official tool aliases (`Bash` -> `shell`, `Read` -> `read`, etc.) so if the model
    // calls the appended official tool instead of `mcp_<12hex>`, we still restore the client's tool name
    for official_name in V2_OFFICIAL_TOOL_ORDER {
        if wire_to_client.contains_key(official_name) {
            continue;
        }
        let candidates = official_tool_candidates(official_name);
        for declared in &declared_client_names {
            let folded_decl = fold_tool_name(declared);
            if candidates
                .iter()
                .any(|cand| cand.eq_ignore_ascii_case(declared) || fold_tool_name(cand) == folded_decl)
            {
                if !official_set.contains(declared.as_str()) {
                    wire_to_client.insert(official_name.to_string(), declared.clone());
                }
                break;
            }
        }
    }

    obj.insert("tools".into(), Value::Array(shaped_tools));
    retarget_tool_choice(obj, &client_to_wire);
    alias_history_tool_calls(obj, &client_to_wire);

    if target_format == WireFormat::OpenAiResponses {
        sanitize_responses_tools(obj);
        if !obj.contains_key("tool_choice") {
            obj.insert("tool_choice".into(), Value::String("auto".into()));
        }
    }

    wire_to_client
}

pub fn inject_reasoning_content(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for msg in messages {
        let Some(mobj) = msg.as_object_mut() else {
            continue;
        };
        if mobj.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if mobj.get("tool_calls").is_some() {
            let has_rc = mobj
                .get("reasoning_content")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            if !has_rc {
                mobj.insert("reasoning_content".into(), Value::String(" ".into()));
            }
        }
    }
}

pub fn normalize_opencode_request_for_lane(
    model_id: &str,
    body: &mut Value,
    target_format: WireFormat,
) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    obj.insert("stream".into(), Value::Bool(true));

    match target_format {
        WireFormat::OpenAiResponses => {
            if model_id.eq_ignore_ascii_case("muse-spark-1.3-contributor-free")
                && let Some(tc) = obj.get("tool_choice")
                && tc.as_str() != Some("auto")
            {
                obj.insert("tool_choice".into(), Value::String("auto".into()));
            }

            if !obj.contains_key("max_output_tokens") {
                if let Some(v) = obj.remove("max_completion_tokens") {
                    obj.insert("max_output_tokens".into(), v);
                } else if let Some(v) = obj.remove("max_tokens") {
                    obj.insert("max_output_tokens".into(), v);
                }
            } else {
                obj.remove("max_completion_tokens");
                obj.remove("max_tokens");
            }

            if let Some(effort_val) = obj.remove("reasoning_effort")
                && let Some(effort_str) = effort_val.as_str()
            {
                let lowered = effort_str.trim().to_ascii_lowercase();
                let norm = match lowered.as_str() {
                    "max" | "ultra" => "xhigh".to_string(),
                    other => other.to_string(),
                };
                if !obj.contains_key("reasoning") && norm != "auto" && norm != "none" {
                    obj.insert(
                        "reasoning".into(),
                        json!({ "effort": norm, "summary": "auto" }),
                    );
                }
            }
            if let Some(reasoning) = obj.get_mut("reasoning").and_then(Value::as_object_mut)
                && !reasoning.contains_key("summary")
            {
                reasoning.insert("summary".into(), Value::String("auto".into()));
            }

            obj.insert("store".into(), Value::Bool(false));
            sanitize_responses_input(obj);
        }
        WireFormat::OpenAiChat => {
            if obj
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .is_some_and(|s| s.eq_ignore_ascii_case("auto"))
            {
                obj.remove("reasoning_effort");
            }
            // Ensure stream_options.include_usage = true (matching OpenCode 2.x CLI)
            match obj.get_mut("stream_options").and_then(Value::as_object_mut) {
                Some(so) => {
                    so.insert("include_usage".into(), Value::Bool(true));
                }
                None => {
                    obj.insert("stream_options".into(), json!({ "include_usage": true }));
                }
            }
            // Ensure max_completion_tokens = 32000 when neither limit was set
            if !obj.contains_key("max_completion_tokens") && !obj.contains_key("max_tokens") {
                obj.insert(
                    "max_completion_tokens".into(),
                    Value::Number(V2_COMPLETION_LIMIT.into()),
                );
            }
            inject_reasoning_content(body);
        }
        WireFormat::AnthropicMessages => {
            if !obj.contains_key("max_tokens") {
                obj.insert(
                    "max_tokens".into(),
                    Value::Number(V2_COMPLETION_LIMIT.into()),
                );
            }
        }
    }
}

/// Serializes an OpenCode anonymous payload in the exact top-level JSON key order
/// produced by Bun's `JSON.stringify` (`model`, `messages`, `tools`, `stream`,
/// `stream_options`, `max_completion_tokens`, then sorted extras) on `OpenAiChat`.
pub fn encode_opencode_body(body: &Value, target_format: WireFormat) -> Result<Bytes> {
    if target_format == WireFormat::OpenAiChat
        && let Some(obj) = body.as_object()
    {
        let mut buf = Vec::with_capacity(4096);
        buf.push(b'{');
        let mut written = HashSet::with_capacity(obj.len());
        let mut first = true;

        let mut write_entry = |key: &str, val: &Value| -> Result<()> {
            if !first {
                buf.push(b',');
            }
            first = false;
            serde_json::to_writer(&mut buf, key)
                .map_err(|e| TesRouteError::Internal(format!("serialize key {key}: {e}")))?;
            buf.push(b':');
            serde_json::to_writer(&mut buf, val)
                .map_err(|e| TesRouteError::Internal(format!("serialize value {key}: {e}")))?;
            Ok(())
        };

        for key in V2_CHAT_KEY_ORDER {
            if let Some(val) = obj.get(key) {
                write_entry(key, val)?;
                written.insert(key.to_string());
            }
        }

        let mut rest: Vec<&String> = obj.keys().filter(|k| !written.contains(*k)).collect();
        rest.sort();
        for key in rest {
            if let Some(val) = obj.get(key) {
                write_entry(key, val)?;
            }
        }
        buf.push(b'}');
        return Ok(Bytes::from(buf));
    }

    let vec = serde_json::to_vec(body)
        .map_err(|e| TesRouteError::Internal(format!("serialize upstream body: {e}")))?;
    Ok(Bytes::from(vec))
}

pub fn is_stale_reasoning_reference(err_body: &str) -> bool {
    let text = err_body.to_ascii_lowercase();
    if !text.contains("reasoning item") && !text.contains("reasoning reference") {
        return false;
    }
    ["not found", "expir", "does not exist", "no longer"]
        .iter()
        .any(|m| text.contains(m))
}

pub fn strip_stale_reasoning_inputs(body: &mut Value) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    if obj.remove("previous_response_id").is_some() {
        changed = true;
    }
    if let Some(arr) = obj.get_mut("input").and_then(Value::as_array_mut) {
        let before = arr.len();
        arr.retain(|item| item.get("type").and_then(Value::as_str) != Some("reasoning"));
        if arr.len() != before {
            changed = true;
        }
    }
    changed
}

fn anthropic_budget_for_effort(effort: &str) -> i64 {
    match effort {
        "minimal" => 1024,
        "low" => 2048,
        "medium" => 8192,
        "high" => 16384,
        "xhigh" => 24576,
        "max" => 31999,
        _ => 8192,
    }
}

fn value_disables_reasoning(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(false)) => true,
        Some(Value::String(s)) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "none" | "disabled" | "off" | "false"
        ),
        Some(Value::Object(m)) => {
            m.get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| matches!(t.trim().to_ascii_lowercase().as_str(), "disabled" | "none"))
                || m.get("effort").and_then(Value::as_str).is_some_and(|e| {
                    matches!(e.trim().to_ascii_lowercase().as_str(), "disabled" | "none")
                })
        }
        _ => false,
    }
}

/// Applies the operator-configured OpenCode Thinking Level (`opencode2api` `ForcedEffort`)
/// when the caller did not explicitly supply one.
pub fn apply_forced_thinking_level(
    target_format: WireFormat,
    body: &mut Value,
    raw_level: &str,
) -> bool {
    apply_thinking_level(target_format, body, raw_level, false)
}

/// Applies an OpenCode Thinking / Power level (`none`, `minimal`, `low`, `medium`, `high`,
/// `xhigh`, `max`) to the upstream payload. When `overwrite_existing` is `true` (e.g. a combo
/// entry explicitly specifies a level like `(high)`), it overrides any inherited fields.
pub fn apply_thinking_level(
    target_format: WireFormat,
    body: &mut Value,
    raw_level: &str,
    overwrite_existing: bool,
) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    let level = raw_level.trim().to_ascii_lowercase();
    if level == "auto" {
        if overwrite_existing {
            match target_format {
                WireFormat::OpenAiChat => {
                    obj.remove("reasoning_effort");
                    obj.remove("reasoning");
                }
                WireFormat::OpenAiResponses => {
                    obj.remove("reasoning");
                    obj.remove("reasoning_effort");
                }
                WireFormat::AnthropicMessages => {
                    obj.remove("thinking");
                    obj.remove("output_config");
                    obj.remove("effort");
                }
            }
        }
        return false;
    }

    if !matches!(
        level.as_str(),
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
    ) {
        return false;
    }

    let disable = level == "none";
    match target_format {
        WireFormat::OpenAiChat => {
            if !overwrite_existing
                && (value_disables_reasoning(obj.get("reasoning_effort"))
                    || value_disables_reasoning(obj.get("reasoning"))
                    || value_disables_reasoning(obj.get("thinking"))
                    || obj
                        .get("reasoning_effort")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty()))
            {
                return false;
            }
            if disable {
                obj.remove("reasoning_effort");
                obj.remove("reasoning");
                return false;
            }
            let wire_level = match level.as_str() {
                "max" | "xhigh" => "high",
                "minimal" => "low",
                other => other,
            };
            obj.insert("reasoning_effort".into(), Value::String(wire_level.to_string()));
            true
        }
        WireFormat::OpenAiResponses => {
            if !overwrite_existing
                && (value_disables_reasoning(obj.get("reasoning"))
                    || value_disables_reasoning(obj.get("reasoning_effort"))
                    || obj
                        .get("reasoning")
                        .and_then(Value::as_object)
                        .and_then(|r| r.get("effort"))
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty()))
            {
                return false;
            }
            if disable {
                obj.remove("reasoning");
                obj.remove("reasoning_effort");
                return false;
            }
            let norm = if level == "max" {
                "xhigh".to_string()
            } else {
                level
            };
            match obj.get_mut("reasoning").and_then(Value::as_object_mut) {
                Some(r) => {
                    r.insert("effort".into(), Value::String(norm));
                }
                None => {
                    obj.insert(
                        "reasoning".into(),
                        json!({ "effort": norm, "summary": "auto" }),
                    );
                }
            }
            true
        }
        WireFormat::AnthropicMessages => {
            if !overwrite_existing
                && (value_disables_reasoning(obj.get("thinking"))
                    || value_disables_reasoning(obj.get("output_config"))
                    || value_disables_reasoning(obj.get("effort"))
                    || obj
                        .get("output_config")
                        .and_then(Value::as_object)
                        .and_then(|o| o.get("effort"))
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                    || obj
                        .get("effort")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty()))
            {
                return false;
            }
            if disable {
                obj.remove("thinking");
                obj.remove("output_config");
                obj.remove("effort");
                return false;
            }
            let budget = anthropic_budget_for_effort(&level);
            obj.insert("output_config".into(), json!({ "effort": level }));
            obj.insert(
                "thinking".into(),
                json!({ "type": "enabled", "budget_tokens": budget }),
            );
            if let Some(cur) = obj.get("max_tokens").and_then(Value::as_i64)
                && cur > 0
                && cur <= budget
            {
                obj.insert("max_tokens".into(), Value::Number((budget + 4096).into()));
            }
            true
        }
    }
}

pub fn strip_forced_thinking_level(target_format: WireFormat, body: &mut Value) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    match target_format {
        WireFormat::OpenAiChat => obj.remove("reasoning_effort").is_some(),
        WireFormat::OpenAiResponses => obj.remove("reasoning").is_some(),
        WireFormat::AnthropicMessages => {
            let a = obj.remove("thinking").is_some();
            let b = obj.remove("output_config").is_some();
            a || b
        }
    }
}

/// Matches `opencode2api/internal/gateway/probe.go` `isEffortRejection`: returns `true` when
/// an HTTP 400/422 error rejects the requested reasoning/thinking effort level.
pub fn is_effort_rejection(status: u16, err_body: &str) -> bool {
    if status != 400 && status != 422 {
        return false;
    }
    let lower = err_body.to_ascii_lowercase();
    if !lower.contains("effort") && !lower.contains("thinking") && !lower.contains("reasoning") {
        return false;
    }
    [
        "invalid",
        "unsupported",
        "not support",
        "not allowed",
        "must be",
        "allowed values",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

fn sanitize_responses_input(obj: &mut Map<String, Value>) {
    if let Some(Value::String(s)) = obj.get("input") {
        let text = s.clone();
        obj.insert(
            "input".into(),
            json!([{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }]),
        );
    }

    let valid_input = match obj.get_mut("input").and_then(Value::as_array_mut) {
        Some(arr) => {
            arr.retain_mut(|item| {
                let Some(iobj) = item.as_object_mut() else {
                    return true;
                };
                if iobj.get("type").and_then(Value::as_str) == Some("reasoning") {
                    return false;
                }
                iobj.remove("encrypted_content");
                iobj.remove("reasoning_encrypted_content");

                match iobj.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        let name = iobj
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .unwrap_or("");
                        if name.is_empty() {
                            return false;
                        }
                        let clamped_name: String = name.chars().take(MAX_TOOL_NAME_LEN).collect();
                        iobj.insert("name".into(), Value::String(clamped_name));
                        if let Some(cid) = iobj.get("call_id").and_then(Value::as_str) {
                            let clamped: String = cid.chars().take(64).collect();
                            iobj.insert("call_id".into(), Value::String(clamped));
                        }
                        if let Some(args) = iobj.get("arguments")
                            && !args.is_string()
                        {
                            iobj.insert("arguments".into(), Value::String(args.to_string()));
                        }
                    }
                    Some("function_call_output") => {
                        if let Some(cid) = iobj.get("call_id").and_then(Value::as_str) {
                            let clamped: String = cid.chars().take(64).collect();
                            iobj.insert("call_id".into(), Value::String(clamped));
                        }
                        if let Some(out) = iobj.get("output")
                            && !out.is_string()
                        {
                            iobj.insert("output".into(), Value::String(out.to_string()));
                        }
                    }
                    _ => {}
                }
                true
            });
            !arr.is_empty()
        }
        None => false,
    };

    if !valid_input {
        obj.insert(
            "input".into(),
            json!([{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "..." }] }]),
        );
    }
}

fn sanitize_responses_tools(obj: &mut Map<String, Value>) {
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        let mut valid_names = HashSet::new();
        tools.retain_mut(|tool| {
            let Some(tobj) = tool.as_object_mut() else {
                return false;
            };
            let fn_obj = tobj.get("function").and_then(Value::as_object).cloned();
            let raw_name = tobj
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| fn_obj.as_ref()?.get("name").and_then(Value::as_str))
                .unwrap_or("")
                .trim();
            if raw_name.is_empty() {
                return false;
            }
            let name: String = raw_name.chars().take(MAX_TOOL_NAME_LEN).collect();
            let desc = tobj
                .get("description")
                .and_then(Value::as_str)
                .or_else(|| fn_obj.as_ref()?.get("description").and_then(Value::as_str))
                .map(str::to_string);
            let mut params = tobj
                .get("parameters")
                .filter(|v| v.is_object())
                .cloned()
                .or_else(|| {
                    fn_obj
                        .as_ref()?
                        .get("parameters")
                        .filter(|v| v.is_object())
                        .cloned()
                })
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            if let Some(pobj) = params.as_object_mut()
                && pobj.get("type").and_then(Value::as_str) == Some("object")
                && !pobj.contains_key("properties")
            {
                pobj.insert("properties".into(), json!({}));
            }
            tobj.clear();
            tobj.insert("type".into(), Value::String("function".into()));
            tobj.insert("name".into(), Value::String(name.clone()));
            if let Some(d) = desc
                && !d.is_empty()
            {
                tobj.insert("description".into(), Value::String(d));
            }
            tobj.insert("parameters".into(), params);
            valid_names.insert(name);
            true
        });

        if let Some(tc) = obj.get("tool_choice").and_then(Value::as_object)
            && tc.get("type").and_then(Value::as_str) == Some("function")
        {
            let target_name = tc
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if target_name.is_empty() || !valid_names.contains(target_name) {
                obj.remove("tool_choice");
            }
        }
    }
}

pub fn restore_tool_names(payload: &mut Value, map: &ToolRenameMap) {
    if map.is_empty() {
        return;
    }
    if let Some(arr) = payload.as_array_mut() {
        for item in arr {
            restore_tool_names(item, map);
        }
        return;
    }
    let Some(obj) = payload.as_object_mut() else {
        return;
    };

    // Claude streaming content_block_start
    if obj.get("type").and_then(Value::as_str) == Some("content_block_start")
        && let Some(cb) = obj.get_mut("content_block").and_then(Value::as_object_mut)
        && cb.get("type").and_then(Value::as_str) == Some("tool_use")
        && let Some(name) = cb.get("name").and_then(Value::as_str)
        && let Some(orig) = map.get(name)
    {
        cb.insert("name".into(), Value::String(orig.clone()));
    }

    // Claude non-streaming message content[]
    if let Some(content) = obj.get_mut("content").and_then(Value::as_array_mut) {
        for block in content {
            if let Some(bobj) = block.as_object_mut()
                && bobj.get("type").and_then(Value::as_str) == Some("tool_use")
                && let Some(name) = bobj.get("name").and_then(Value::as_str)
                && let Some(orig) = map.get(name)
            {
                bobj.insert("name".into(), Value::String(orig.clone()));
            }
        }
    }

    // OpenAI Chat Completions (choices[].delta / choices[].message)
    if let Some(choices) = obj.get_mut("choices").and_then(Value::as_array_mut) {
        for choice in choices {
            let Some(cobj) = choice.as_object_mut() else {
                continue;
            };
            for holder in ["delta", "message"] {
                if let Some(calls) = cobj
                    .get_mut(holder)
                    .and_then(Value::as_object_mut)
                    .and_then(|h| h.get_mut("tool_calls"))
                    .and_then(Value::as_array_mut)
                {
                    for call in calls {
                        if let Some(fn_obj) = call
                            .as_object_mut()
                            .and_then(|c| c.get_mut("function"))
                            .and_then(Value::as_object_mut)
                            && let Some(name) = fn_obj.get("name").and_then(Value::as_str)
                            && let Some(orig) = map.get(name)
                        {
                            fn_obj.insert("name".into(), Value::String(orig.clone()));
                        }
                    }
                }
            }
        }
    }

    // OpenAI Responses final body output[]
    if let Some(output) = obj.get_mut("output").and_then(Value::as_array_mut) {
        for item in output {
            if let Some(iobj) = item.as_object_mut()
                && iobj.get("type").and_then(Value::as_str) == Some("function_call")
                && let Some(name) = iobj.get("name").and_then(Value::as_str)
                && let Some(orig) = map.get(name)
            {
                iobj.insert("name".into(), Value::String(orig.clone()));
            }
        }
    }

    // OpenAI Responses SSE item
    if let Some(item) = obj.get_mut("item").and_then(Value::as_object_mut)
        && item.get("type").and_then(Value::as_str) == Some("function_call")
        && let Some(name) = item.get("name").and_then(Value::as_str)
        && let Some(orig) = map.get(name)
    {
        item.insert("name".into(), Value::String(orig.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conceals_all_client_tools_appends_13_v2_tools_and_orders_chat_keys() {
        let mut body = json!({
            "model": "deepseek-v4.1-flash-free",
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": "calling tool", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "playwright_browser_run_code_unsafe", "arguments": "{}" } }] }
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "playwright_browser_run_code_unsafe",
                        "description": "run code",
                        "parameters": { "type": "object", "properties": {} }
                    }
                }
            ]
        });

        normalize_opencode_request_for_lane("deepseek-v4.1-flash-free", &mut body, WireFormat::OpenAiChat);
        let map = apply_fingerprint_tools(&mut body, WireFormat::OpenAiChat);

        let tools = body["tools"].as_array().unwrap();
        // 1 renamed client tool + 13 official v2 CLI tools = 14 tools
        assert_eq!(tools.len(), 14);

        let wire_name = tools[0]["function"]["name"].as_str().unwrap().to_string();
        assert!(wire_name.starts_with("mcp_"));
        assert_eq!(
            map.get(&wire_name).map(String::as_str),
            Some("playwright_browser_run_code_unsafe")
        );

        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        for official in V2_OFFICIAL_TOOL_ORDER {
            assert!(names.contains(&official), "missing official tool {official}");
        }

        let sys_content = body["messages"][0]["content"].as_str().unwrap();
        assert!(sys_content.contains(OPENCODE_PROMPT_MARKER));
        assert!(sys_content.contains("<env>"));
        assert!(sys_content.contains("<available_skills>"));

        let encoded = encode_opencode_body(&body, WireFormat::OpenAiChat).unwrap();
        let encoded_str = std::str::from_utf8(&encoded).unwrap();
        assert!(encoded_str.starts_with(r#"{"model":"deepseek-v4.1-flash-free","messages":"#));

        let mut chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": { "name": wire_name, "arguments": "{}" }
                    }]
                }
            }]
        });
        restore_tool_names(&mut chunk, &map);
        assert_eq!(
            chunk["choices"][0]["delta"]["tool_calls"][0]["function"]["name"].as_str(),
            Some("playwright_browser_run_code_unsafe")
        );
    }

    #[test]
    fn skips_tool_injection_on_opencode_v2_title_requests() {
        let mut title_body = json!({
            "model": "mimo-v2.6-flash-free",
            "messages": [
                {
                    "role": "system",
                    "content": "You decide whether the session title still matches the conversation. Reply with a title and nothing else."
                },
                {
                    "role": "user",
                    "content": "Fix the proxy pool selector"
                }
            ]
        });

        assert!(is_opencode_title_request(&title_body));
        let map = apply_fingerprint_tools(&mut title_body, WireFormat::OpenAiChat);
        assert!(map.is_empty());
        assert!(
            title_body.get("tools").is_none(),
            "title requests must NEVER carry injected agent tools"
        );
        assert_eq!(
            synthesize_title_from_body(&title_body),
            "Fix the proxy pool selector"
        );
    }
}
