use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use bytes::Bytes;
use regex::Regex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tesie_core::{Result, TesRouteError};
use tesie_store::now_ms;

/// Official fallback thought signature from `luqman-v1/9router-go` (`DefaultThinkingSignature`)
/// required when replaying tool calls across Gemini/Claude thinking turns without an explicit `__ts__` token.
pub const DEFAULT_THINKING_SIGNATURE: &str = "EuwGCukGAXLI2nxwZIq54WWSoL/YN0P3TsDZ7zRnLi8g0S4aVr2HUGxvaHKySuY6HAVzcE0GPGjXrytLIldxthSvfxgUlJh6Qa9Z+Oj5QZBlYdg6HaJ6yuY5R7waE6rdwBsRf7Ft2j3DJ9rMi9qhWFqApewYtPhls3VHtuvND3l8Rm09+lbAXQs6KKWEWrxNLKTBkfpMgXhRERc/TQRMZu1twAablm6/Zk1tsYRvfWKLsNbeKF+CCojJdXJKvnR/8Ouuoa+Y2Ti20hcW7aZIIjZDFYPU//k6Ybmhg69J/imbFai2ckhfLaisqdDkdoIiBJScTOUvYqP6AE9d4MsydSC+UlhIMk4hoP76R8vUSCZRMkjOaDXstf/QoVZKbt94wyRZgAJ1G0BqI8L5ow86kLpA4wJEtxsRGymOE4bKUvApveBakYDNM9APkf+LbtbzWSseGjoZcSlycF9iN8Q2XNYKRrHbv3Lr5Y8JjdH/5y/6SHkNehTEZugaeGnSPSyCTWto1kQgHpxdWmhkLfJGNUGLmue7Mesj4TSms4J33mRpYVhNB/J333FCqIP0hr/E7BkkjEn7yZ4X7SQlh+xKPurapsnHRwiKmtsilmEFrnTE9iQr+pMr6M29qqFNv1tr5yumbaJw8JW9sB15tNsRv+dW6BjNanbsKz7HCgKUBc8tGy+7YuhXzAfViyRefcjK7eZW0Fbyt7AbybJTKz78W8NH7ye6LAwzOebXpeZ4D43fNIt8bKh26qgduSQv/7o+pAflkuqHZ99YWgHQ8h8OkZFi3eOiSYjsjhdZ/czWOdoPI/OnqIldzMPF5YlrKBLFX8VhRKVmqgsmWf5PHGulHhMkVlS+XG2UIseGy69ARa93D78Gsa+1n1kJr7EEB7Rh+27vUMxVYLdz1yMSvE5nalTAlg/ZeG8+XQ0cHuAI3KbQpHW2Q++RdXfm5JzD5WdJZUU+Zn8t8UUn85BH4RxZLeE0qJikgSsKoYVBc6YhiMjhPgkR95ReimY4Z0xCJdRo1gjexOFeODZMpQF6Yxnoic7IrdgsFA3iePTbFnPp3IAM1fAThWhXJUn3QInUOTd5o1qmTmn6REbL15g/JQNl+dqUoPkhleeb2V3kjqp1okmO3wMZbPknR3S1LZNmlS72/iBQUm+n2b/RCn4PjmM2";

const MAX_ANTIGRAVITY_OUTPUT_TOKENS: i64 = 65_536;
const DECOY_UNAVAILABLE_DESC: &str = "This tool is currently unavailable.";

const ANTIGRAVITY_NATIVE_TOOLS: &[&str] = &[
    "browser_subagent",
    "command_status",
    "find_by_name",
    "generate_image",
    "grep_search",
    "list_dir",
    "list_resources",
    "mcp_sequential-thinking_sequentialthinking",
    "multi_replace_file_content",
    "notify_user",
    "read_resource",
    "read_terminal",
    "read_url_content",
    "replace_file_content",
    "run_command",
    "search_web",
    "send_command_input",
    "task_boundary",
    "view_content_chunk",
    "view_file",
    "write_to_file",
];

static OPENCODE_WORD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)opencode").unwrap());
static CONVENTIONS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<system[-_]conventions\b[^>]*>.*?</system[-_]conventions\s*>").unwrap());
static BILLING_HDR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^x-anthropic-billing-header:[^\n]*(?:\r?\n)*").unwrap());

/// Resolves a user/catalog model ID + optional reasoning effort to the exact Cloud Code wire model ID
/// (ported from `luqman-v1/9router-go/internal/translator/antigravity.go`).
pub fn resolve_antigravity_upstream_model(raw_model: &str, effort: Option<&str>) -> String {
    let without_effort = raw_model.split('(').next().unwrap_or(raw_model).trim();
    let base = without_effort
        .strip_prefix("ag/")
        .or_else(|| without_effort.strip_prefix("AG/"))
        .unwrap_or(without_effort)
        .trim()
        .to_ascii_lowercase();

    let tier_candidates: &[&str] = match effort
        .map(|e| e.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("xhigh") | Some("max") => &["xhigh", "high"],
        Some("high") => &["high"],
        Some("medium") => &["medium"],
        Some("minimal") | Some("low") => &["low"],
        _ => &[],
    };

    for t in tier_candidates {
        let candidate = format!("{base}-{t}");
        if let Some(mapped) = map_known_antigravity_id(&candidate) {
            return mapped.to_string();
        }
    }

    if let Some(mapped) = map_known_antigravity_id(&base) {
        return mapped.to_string();
    }

    base
}

fn max_output_tokens_for_antigravity_wire(wire_model: &str) -> i64 {
    let lower = wire_model.to_ascii_lowercase();
    if lower.starts_with("gemini-4-neon")
        || lower.starts_with("gemini-4-hydrogen")
        || lower.starts_with("gemini-4-carbon")
    {
        262_144
    } else if lower.starts_with("claude-opus-5-5") || lower.starts_with("claude-sonnet-5-5") {
        128_000
    } else if lower.starts_with("claude-opus-4-6") || lower.starts_with("claude-sonnet-4-6") {
        64_000
    } else if lower.starts_with("gpt-oss-120b") {
        32_768
    } else {
        MAX_ANTIGRAVITY_OUTPUT_TOKENS
    }
}

fn map_known_antigravity_id(id: &str) -> Option<&'static str> {
    match id {
        "carbon" | "gemini-4-carbon" => Some("gemini-4-carbon-medium"),
        "carbon-high" | "gemini-4-carbon-high" => Some("gemini-4-carbon-high"),
        "carbon-medium" | "gemini-4-carbon-medium" => Some("gemini-4-carbon-medium"),
        "carbon-low" | "gemini-4-carbon-low" => Some("gemini-4-carbon-low"),
        "neon" | "gemini-4-neon" | "neon-medium" | "gemini-4-neon-medium" => {
            Some("gemini-4-neon-medium")
        }
        "neon-high" | "gemini-4-neon-high" => Some("gemini-4-neon-high"),
        "neon-xhigh" | "gemini-4-neon-xhigh" => Some("gemini-4-neon-xhigh"),
        "neon-low" | "gemini-4-neon-low" => Some("gemini-4-neon-low"),
        "argon" | "gemini-4-argon" => Some("gemini-4-argon-medium"),
        "argon-high" | "gemini-4-argon-high" => Some("gemini-4-argon-high"),
        "argon-medium" | "gemini-4-argon-medium" => Some("gemini-4-argon-medium"),
        "argon-low" | "gemini-4-argon-low" => Some("gemini-4-argon-low"),
        "hydrogen" | "gemini-4-hydrogen" => Some("gemini-4-hydrogen-medium"),
        "hydrogen-xhigh" | "gemini-4-hydrogen-xhigh" => Some("gemini-4-hydrogen-xhigh"),
        "hydrogen-high" | "gemini-4-hydrogen-high" => Some("gemini-4-hydrogen-high"),
        "hydrogen-medium" | "gemini-4-hydrogen-medium" => Some("gemini-4-hydrogen-medium"),
        "hydrogen-low" | "gemini-4-hydrogen-low" => Some("gemini-4-hydrogen-low"),
        "orchestrator-carbon"
        | "orchestrator-carbon-high"
        | "orchestrator-carbon-medium"
        | "orchestrator-carbon-low" => Some("orchestrator-carbon"),
        "orchestrator-argon"
        | "orchestrator-argon-high"
        | "orchestrator-argon-medium"
        | "orchestrator-argon-low" => Some("orchestrator-argon"),
        "gemini-3.8-flash" => Some("gemini-3.8-flash-medium"),
        "gemini-3.8-flash-high" => Some("gemini-3.8-flash-high"),
        "gemini-3.8-flash-medium" => Some("gemini-3.8-flash-medium"),
        "gemini-3.8-flash-low" => Some("gemini-3.8-flash-low"),
        "claude-opus-5-5"
        | "claude-opus-5.5"
        | "claude-opus-5-5-thinking"
        | "claude-opus-5-5-medium"
        | "claude-opus-5.5-medium" => Some("claude-opus-5-5-medium"),
        "claude-opus-5-5-high" | "claude-opus-5.5-high" => Some("claude-opus-5-5-high"),
        "claude-opus-5-5-low" | "claude-opus-5.5-low" => Some("claude-opus-5-5-low"),
        "claude-sonnet-5-5"
        | "claude-sonnet-5.5"
        | "claude-sonnet-5-5-thinking"
        | "claude-sonnet-5-5-medium"
        | "claude-sonnet-5.5-medium" => Some("claude-sonnet-5-5-medium"),
        "claude-sonnet-5-5-high" | "claude-sonnet-5.5-high" => Some("claude-sonnet-5-5-high"),
        "claude-sonnet-5-5-low" | "claude-sonnet-5.5-low" => Some("claude-sonnet-5-5-low"),
        "claude-sonnet-4-6"
        | "claude-sonnet-4.6"
        | "claude-sonnet-4-6-thinking"
        | "claude-sonnet-4-6-high"
        | "claude-sonnet-4-6-medium"
        | "claude-sonnet-4-6-low" => Some("claude-sonnet-4-6"),
        "claude-opus-4-6"
        | "claude-opus-4.6"
        | "claude-opus-4-6-thinking"
        | "claude-opus-4-6-high"
        | "claude-opus-4-6-medium"
        | "claude-opus-4-6-low" => Some("claude-opus-4-6-thinking"),
        "gpt-oss-120b"
        | "gpt-oss-120b-medium"
        | "gpt-oss-120b-high"
        | "gpt-oss-120b-low" => Some("gpt-oss-120b-medium"),
        "gemini-3-flash"
        | "gemini-3-flash-high"
        | "gemini-3-flash-medium"
        | "gemini-3-flash-low" => Some("gemini-3-flash"),
        "gemini-3.5-flash-lite"
        | "gemini-3.5-flash-lite-high"
        | "gemini-3.5-flash-lite-medium"
        | "gemini-3.5-flash-lite-low" => Some("gemini-3.5-flash-lite"),
        "gemini-3.1-flash-lite"
        | "gemini-3.1-flash-lite-high"
        | "gemini-3.1-flash-lite-medium"
        | "gemini-3.1-flash-lite-low" => Some("gemini-3.1-flash-lite"),
        "gemini-2.5-pro"
        | "gemini-2.5-pro-high"
        | "gemini-2.5-pro-medium"
        | "gemini-2.5-pro-low" => Some("gemini-2.5-pro"),
        "gemini-2.5-flash-lite" => Some("gemini-2.5-flash-lite"),
        "gemini-2.5-flash" => Some("gemini-2.5-flash"),
        "gemini-2.5-flash-high" | "gemini-2.5-flash-thinking" => Some("gemini-2.5-flash-thinking"),
        "gemini-2.5-flash-medium" | "gemini-2.5-flash-low" => Some("gemini-2.5-flash"),
        _ => None,
    }
}

pub fn sanitize_competitive_prompt(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut s = text.replace(
        "You are a Claude agent, built on Anthropic's Claude Agent SDK.",
        "You are an AI coding assistant.",
    );
    s = s.replace("Anthropic's Claude Agent SDK", "an AI SDK");
    s = s.replace("<available_skills>", "<available_guides>");
    s = s.replace("</available_skills>", "</available_guides>");
    s = CONVENTIONS_RE
        .replace_all(
            &s,
            "<guidelines>\nRequirement terminology: MUST means mandatory, SHOULD means recommended, and MAY means optional.\n</guidelines>",
        )
        .into_owned();
    s = BILLING_HDR_RE.replace_all(&s, "").into_owned();
    s = OPENCODE_WORD_RE.replace_all(&s, "coding-cli").into_owned();
    s
}

fn uuid_from_seed(seed: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&hasher.finalize()[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex_s = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &hex_s[0..8],
        &hex_s[8..12],
        &hex_s[12..16],
        &hex_s[16..20],
        &hex_s[20..32]
    )
}

pub fn numeric_session_id(session_seed: &str) -> String {
    let s = session_seed.trim();
    if s.is_empty() || s.len() > 256 {
        return String::new();
    }
    if s.chars().enumerate().all(|(i, c)| c.is_ascii_digit() || (i == 0 && c == '-')) {
        return s.to_string();
    }
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let sum = hasher.finalize();
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&sum[..8]);
    let n = u64::from_be_bytes(arr) & 0x7fff_ffff_ffff_ffff;
    format!("-{n}")
}

fn build_antigravity_request_id(session_id: &str, model: &str, content_count: usize) -> String {
    let sess = if session_id.is_empty() {
        "anonymous"
    } else {
        session_id
    };
    let conv = uuid_from_seed(&format!("antigravity:conversation:{sess}"));
    let traj = uuid_from_seed(&format!("antigravity:trajectory:{sess}:{model}:agent"));
    let step = (content_count * 2).saturating_sub(1).max(1);
    format!("agent/{conv}/{}/{traj}/{step}", now_ms())
}

pub fn clean_json_schema(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let mut effective_map = map.clone();

            // 1. Convert `oneOf` to `anyOf` if `anyOf` is absent
            if let Some(one_of) = effective_map.remove("oneOf") {
                if !effective_map.contains_key("anyOf") {
                    effective_map.insert("anyOf".to_string(), one_of);
                }
            }

            // 2. Flatten `allOf`: merge any object properties into this schema
            if let Some(all_of) = effective_map.remove("allOf") {
                if let Value::Array(all_arr) = all_of {
                    for sub in all_arr {
                        if let Value::Object(sub_map) = sub {
                            for (k, v) in sub_map {
                                if k == "properties" {
                                    if let Value::Object(sub_props) = v {
                                        let current_props = effective_map
                                            .entry("properties".to_string())
                                            .or_insert_with(|| Value::Object(Map::new()));
                                        if let Value::Object(cp_map) = current_props {
                                            for (pk, pv) in sub_props {
                                                cp_map.entry(pk).or_insert(pv);
                                            }
                                        }
                                    }
                                } else if k == "required" {
                                    if let Value::Array(sub_req) = v {
                                        let current_req = effective_map
                                            .entry("required".to_string())
                                            .or_insert_with(|| Value::Array(Vec::new()));
                                        if let Value::Array(cr_arr) = current_req {
                                            for r in sub_req {
                                                if !cr_arr.contains(&r) {
                                                    cr_arr.push(r);
                                                }
                                            }
                                        }
                                    }
                                } else if !effective_map.contains_key(&k) {
                                    effective_map.insert(k, v);
                                }
                            }
                        }
                    }
                }
            }

            // 3. Handle `const`: Gemini Schema protobuf has NO `const` field.
            // Convert `const: X` into `enum: [X]` and infer scalar `type` if missing.
            if let Some(const_val) = effective_map.remove("const") {
                if !effective_map.contains_key("enum") {
                    let str_val = match &const_val {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        other => other.to_string().trim_matches('"').to_string(),
                    };
                    effective_map.insert("enum".to_string(), json!([str_val]));
                }
                if !effective_map.contains_key("type") {
                    let inferred_type = match &const_val {
                        Value::String(_) => "string",
                        Value::Number(n) => {
                            if n.is_i64() || n.is_u64() {
                                "integer"
                            } else {
                                "number"
                            }
                        }
                        Value::Bool(_) => "boolean",
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                        Value::Null => "string",
                    };
                    effective_map.insert("type".to_string(), json!(inferred_type));
                }
            }

            let mut out = Map::new();
            let mut is_nullable = effective_map
                .get("nullable")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            // 4. Handle `type`: In Gemini proto, `type` is a single enum (scalar), NEVER an array/list.
            // E.g. `type: ["string", "null"]` -> `type: "string", nullable: true`.
            if let Some(type_val) = effective_map.remove("type") {
                match type_val {
                    Value::String(s) => {
                        let lower = s.to_ascii_lowercase();
                        if lower == "null" {
                            is_nullable = true;
                            out.insert("type".to_string(), json!("string"));
                        } else {
                            out.insert("type".to_string(), json!(lower));
                        }
                    }
                    Value::Array(type_arr) => {
                        let mut chosen_type = None;
                        for t in type_arr {
                            if let Some(ts) = t.as_str() {
                                let t_lower = ts.to_ascii_lowercase();
                                if t_lower == "null" {
                                    is_nullable = true;
                                } else if chosen_type.is_none() {
                                    chosen_type = Some(t_lower);
                                }
                            }
                        }
                        let final_type = chosen_type.unwrap_or_else(|| "string".to_string());
                        out.insert("type".to_string(), json!(final_type));
                    }
                    _ => {
                        out.insert("type".to_string(), json!("string"));
                    }
                }
            }

            // 5. Handle `items`: In Gemini proto, `items` is a single Schema object, NEVER an array.
            // E.g. `items: [{ "type": "string" }]` -> `items: { "type": "string" }`.
            if let Some(items_val) = effective_map.remove("items") {
                let cleaned_items = match items_val {
                    Value::Array(items_arr) => {
                        if items_arr.is_empty() {
                            json!({ "type": "string" })
                        } else if items_arr.len() == 1 {
                            clean_json_schema(&items_arr[0])
                        } else {
                            json!({
                                "anyOf": items_arr.iter().map(clean_json_schema).collect::<Vec<_>>()
                            })
                        }
                    }
                    Value::Object(_) => clean_json_schema(&items_val),
                    _ => json!({ "type": "string" }),
                };
                out.insert("items".to_string(), cleaned_items);
            }

            // 6. Handle `properties`: object mapping property names to Schema objects
            if let Some(props_val) = effective_map.remove("properties") {
                if let Value::Object(props_map) = props_val {
                    let mut cleaned_props = Map::new();
                    for (k, v) in props_map {
                        cleaned_props.insert(k, clean_json_schema(&v));
                    }
                    out.insert("properties".to_string(), Value::Object(cleaned_props));
                }
            }

            // 7. Handle `required`: list of required property string names
            if let Some(req_val) = effective_map.remove("required") {
                if let Value::Array(req_arr) = req_val {
                    let filtered_req: Vec<Value> = req_arr
                        .into_iter()
                        .filter(|v| v.is_string())
                        .collect();
                    if !filtered_req.is_empty() {
                        out.insert("required".to_string(), Value::Array(filtered_req));
                    }
                }
            }

            // 8. Handle `enum`: array of strings
            if let Some(enum_val) = effective_map.remove("enum") {
                if let Value::Array(enum_arr) = enum_val {
                    let str_enum: Vec<Value> = enum_arr
                        .into_iter()
                        .map(|v| match v {
                            Value::String(s) => Value::String(s),
                            other => Value::String(other.to_string().trim_matches('"').to_string()),
                        })
                        .collect();
                    if !str_enum.is_empty() {
                        out.insert("enum".to_string(), Value::Array(str_enum));
                    }
                }
            }

            // 9. Handle `anyOf`: array of Schema objects
            if let Some(any_val) = effective_map.remove("anyOf") {
                if let Value::Array(any_arr) = any_val {
                    let mut can_collapse = true;
                    let mut collapsed_enums = Vec::new();
                    let mut collapsed_type = None;

                    let mut cleaned_any = Vec::new();
                    for branch in any_arr {
                        let cleaned_branch = clean_json_schema(&branch);
                        if let Value::Object(b_obj) = &cleaned_branch {
                            if let Some(b_enum) = b_obj.get("enum").and_then(Value::as_array) {
                                if b_obj.len() <= 2 {
                                    if let Some(t) = b_obj.get("type").and_then(Value::as_str) {
                                        if collapsed_type.is_none() {
                                            collapsed_type = Some(t.to_string());
                                        }
                                    }
                                    for e in b_enum {
                                        if !collapsed_enums.contains(e) {
                                            collapsed_enums.push(e.clone());
                                        }
                                    }
                                } else {
                                    can_collapse = false;
                                }
                            } else {
                                can_collapse = false;
                            }
                        } else {
                            can_collapse = false;
                        }
                        cleaned_any.push(cleaned_branch);
                    }

                    // Collapse anyOf: [{enum: [A]}, {enum: [B]}] into single enum: [A, B]
                    if can_collapse && !collapsed_enums.is_empty() && !out.contains_key("enum") {
                        if !out.contains_key("type") {
                            out.insert(
                                "type".to_string(),
                                json!(collapsed_type.unwrap_or_else(|| "string".to_string())),
                            );
                        }
                        out.insert("enum".to_string(), Value::Array(collapsed_enums));
                    } else if !cleaned_any.is_empty() {
                        out.insert("anyOf".to_string(), Value::Array(cleaned_any));
                    }
                }
            }

            // 10. Handle `description`
            if let Some(desc_val) = effective_map.remove("description") {
                if let Value::String(s) = desc_val {
                    if !s.trim().is_empty() {
                        out.insert("description".to_string(), Value::String(s));
                    }
                }
            }

            // 11. Handle `format`
            if let Some(fmt_val) = effective_map.remove("format") {
                if let Value::String(s) = fmt_val {
                    out.insert("format".to_string(), Value::String(s));
                }
            }

            // 12. Handle `propertyOrdering`
            if let Some(po_val) = effective_map.remove("propertyOrdering") {
                if let Value::Array(po_arr) = po_val {
                    out.insert("propertyOrdering".to_string(), Value::Array(po_arr));
                }
            }

            if is_nullable {
                out.insert("nullable".to_string(), Value::Bool(true));
            }

            // Ensure structural integrity:
            if out.contains_key("properties") && !out.contains_key("type") {
                out.insert("type".to_string(), json!("object"));
            }
            if out.get("type").and_then(Value::as_str) == Some("array") && !out.contains_key("items") {
                out.insert("items".to_string(), json!({ "type": "string" }));
            }
            if !out.contains_key("type") && !out.contains_key("anyOf") {
                out.insert("type".to_string(), json!("string"));
            }

            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(clean_json_schema).collect()),
        other => other.clone(),
    }
}

fn extract_text_parts(content: Option<&Value>) -> Vec<Value> {
    let Some(c) = content else {
        return Vec::new();
    };
    if let Some(s) = c.as_str() {
        if s.is_empty() {
            return Vec::new();
        }
        return vec![json!({ "text": s })];
    }
    if let Some(arr) = c.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("text");
            match item_type {
                "text" | "input_text" | "output_text" => {
                    if let Some(t) = item.get("text").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        parts.push(json!({ "text": t }));
                    }
                }
                "image_url" => {
                    if let Some(url) = item
                        .get("image_url")
                        .and_then(|u| u.get("url").and_then(Value::as_str).or_else(|| u.as_str()))
                        && let Some(rest) = url.strip_prefix("data:")
                        && let Some((mime_part, b64)) = rest.split_once(";base64,")
                    {
                        parts.push(json!({
                            "inlineData": {
                                "mimeType": mime_part,
                                "data": b64
                            }
                        }));
                    }
                }
                _ => {}
            }
        }
        return parts;
    }
    Vec::new()
}

fn split_tool_call_id_and_sig(raw_id: &str) -> (String, Option<String>) {
    if let Some((clean_id, sig)) = raw_id.split_once("__ts__")
        && !sig.is_empty()
    {
        return (clean_id.to_string(), Some(sig.to_string()));
    }
    (raw_id.to_string(), None)
}

fn cloak_tool_name(name: &str, native_set: &HashSet<&str>) -> String {
    if native_set.contains(name) || name.ends_with("_ide") {
        name.to_string()
    } else {
        format!("{name}_ide")
    }
}

pub fn uncloak_tool_name(name: &str) -> String {
    let trimmed = name.strip_prefix("proxy_").unwrap_or(name);
    trimmed
        .strip_suffix("_ide")
        .unwrap_or(trimmed)
        .to_string()
}

/// Builds the full wrapped Antigravity `v1internal:streamGenerateContent?alt=sse` JSON payload
/// from a normalized OpenAI Chat request body.
pub fn build_antigravity_envelope(
    chat_body: &Value,
    project_id: &str,
    wire_model: &str,
    session_seed: &str,
) -> Result<Bytes> {
    let native_set: HashSet<&str> = ANTIGRAVITY_NATIVE_TOOLS.iter().copied().collect();
    let has_tools = chat_body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());

    let mut sys_chunks: Vec<String> = Vec::new();
    let mut raw_contents: Vec<(String, Vec<Value>)> = Vec::new();
    let mut call_id_to_name: HashMap<String, String> = HashMap::new();

    if let Some(messages) = chat_body.get("messages").and_then(Value::as_array) {
        for msg in messages {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
            match role {
                "system" | "developer" => {
                    for p in extract_text_parts(msg.get("content")) {
                        if let Some(t) = p.get("text").and_then(Value::as_str) {
                            let cleaned = sanitize_competitive_prompt(t);
                            if !cleaned.trim().is_empty() {
                                sys_chunks.push(cleaned);
                            }
                        }
                    }
                }
                "assistant" => {
                    let mut parts = extract_text_parts(msg.get("content"));
                    let mut first_fc = true;
                    if let Some(tcs) = msg.get("tool_calls").and_then(Value::as_array) {
                        for tc in tcs {
                            let raw_id = tc
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("call_1");
                            let (clean_id, maybe_sig) = split_tool_call_id_and_sig(raw_id);
                            let orig_name = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(Value::as_str)
                                .unwrap_or("tool");
                            let wire_name = if has_tools {
                                cloak_tool_name(orig_name, &native_set)
                            } else {
                                orig_name.to_string()
                            };
                            call_id_to_name.insert(raw_id.to_string(), wire_name.clone());
                            call_id_to_name.insert(clean_id.clone(), wire_name.clone());

                            let args_val = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(Value::as_str)
                                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                                .filter(Value::is_object)
                                .unwrap_or_else(|| json!({}));

                            let sig = maybe_sig.or_else(|| {
                                if first_fc {
                                    Some(DEFAULT_THINKING_SIGNATURE.to_string())
                                } else {
                                    None
                                }
                            });
                            first_fc = false;

                            let mut fc_part = json!({
                                "functionCall": {
                                    "id": clean_id,
                                    "name": wire_name,
                                    "args": args_val
                                }
                            });
                            if let Some(s) = sig {
                                fc_part["thoughtSignature"] = Value::String(s);
                            }
                            parts.push(fc_part);
                        }
                    }
                    if !parts.is_empty() {
                        raw_contents.push(("model".to_string(), parts));
                    }
                }
                "tool" => {
                    let raw_id = msg
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_1");
                    let (clean_id, _) = split_tool_call_id_and_sig(raw_id);
                    let fn_name = call_id_to_name
                        .get(raw_id)
                        .or_else(|| call_id_to_name.get(&clean_id))
                        .cloned()
                        .or_else(|| {
                            msg.get("name")
                                .and_then(Value::as_str)
                                .map(|n| if has_tools { cloak_tool_name(n, &native_set) } else { n.to_string() })
                        })
                        .unwrap_or_else(|| "tool".to_string());

                    let result_str = match msg.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => serde_json::to_string(other).unwrap_or_default(),
                        None => String::new(),
                    };
                    raw_contents.push((
                        "user".to_string(),
                        vec![json!({
                            "functionResponse": {
                                "id": clean_id,
                                "name": fn_name,
                                "response": {
                                    "result": result_str
                                }
                            }
                        })],
                    ));
                }
                _ => {
                    let parts = extract_text_parts(msg.get("content"));
                    if !parts.is_empty() {
                        raw_contents.push(("user".to_string(), parts));
                    }
                }
            }
        }
    }

    // Merge consecutive same-role turns and bracket with user turns (Gemini requirement)
    let mut merged_contents: Vec<Value> = Vec::new();
    for (role, parts) in raw_contents {
        if let Some(last) = merged_contents.last_mut()
            && last.get("role").and_then(Value::as_str) == Some(role.as_str())
            && let Some(existing_parts) = last.get_mut("parts").and_then(Value::as_array_mut)
        {
            existing_parts.extend(parts);
            continue;
        }
        merged_contents.push(json!({
            "role": role,
            "parts": parts
        }));
    }

    if merged_contents.is_empty() {
        merged_contents.push(json!({
            "role": "user",
            "parts": [{ "text": "..." }]
        }));
    } else {
        if merged_contents[0].get("role").and_then(Value::as_str) != Some("user") {
            merged_contents.insert(
                0,
                json!({
                    "role": "user",
                    "parts": [{ "text": "..." }]
                }),
            );
        }
        if merged_contents
            .last()
            .and_then(|c| c.get("role"))
            .and_then(Value::as_str)
            == Some("model")
        {
            merged_contents.push(json!({
                "role": "user",
                "parts": [{ "text": "Continue." }]
            }));
        }
    }

    let content_count = merged_contents.len();
    let mut inner_req = Map::new();
    inner_req.insert("contents".into(), Value::Array(merged_contents));

    if !sys_chunks.is_empty() {
        inner_req.insert(
            "systemInstruction".into(),
            json!({
                "role": "user",
                "parts": [{ "text": sys_chunks.join("\n\n") }]
            }),
        );
    }

    // Cloak client tools (`_ide` suffix) + append 21 official Antigravity decoy tools
    if let Some(tools_arr) = chat_body.get("tools").and_then(Value::as_array)
        && !tools_arr.is_empty()
    {
        let mut seen = HashSet::new();
        let mut decls = Vec::with_capacity(tools_arr.len() + ANTIGRAVITY_NATIVE_TOOLS.len());

        for t in tools_arr {
            let fn_obj = t.get("function").unwrap_or(t);
            let Some(orig_name) = fn_obj.get("name").and_then(Value::as_str).map(str::trim) else {
                continue;
            };
            if orig_name.is_empty() {
                continue;
            }
            let wire_name = cloak_tool_name(orig_name, &native_set);
            if !seen.insert(wire_name.clone()) {
                continue;
            }
            let desc = fn_obj
                .get("description")
                .and_then(Value::as_str)
                .map(sanitize_competitive_prompt)
                .unwrap_or_default();
            let params = fn_obj
                .get("parameters")
                .or_else(|| fn_obj.get("input_schema"))
                .map(clean_json_schema)
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            decls.push(json!({
                "name": wire_name,
                "description": desc,
                "parameters": params
            }));
        }

        let decoy_params = json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Brief explanation of why you are calling this tool"
                }
            },
            "required": ["reason"]
        });
        for &decoy_name in ANTIGRAVITY_NATIVE_TOOLS {
            if seen.insert(decoy_name.to_string()) {
                decls.push(json!({
                    "name": decoy_name,
                    "description": DECOY_UNAVAILABLE_DESC,
                    "parameters": decoy_params
                }));
            }
        }

        inner_req.insert(
            "tools".into(),
            json!([{ "functionDeclarations": decls }]),
        );
        inner_req.insert(
            "toolConfig".into(),
            json!({
                "functionCallingConfig": {
                    "mode": "VALIDATED"
                }
            }),
        );
    }

    let max_out = chat_body
        .get("max_completion_tokens")
        .or_else(|| chat_body.get("max_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(16_384)
        .clamp(1, max_output_tokens_for_antigravity_wire(wire_model));

    let mut gen_cfg = Map::new();
    gen_cfg.insert("maxOutputTokens".into(), Value::Number(max_out.into()));
    if let Some(temp) = chat_body.get("temperature").and_then(Value::as_f64) {
        gen_cfg.insert("temperature".into(), json!(temp));
    }
    if let Some(top_p) = chat_body.get("top_p").and_then(Value::as_f64) {
        gen_cfg.insert("topP".into(), json!(top_p));
    }
    inner_req.insert("generationConfig".into(), Value::Object(gen_cfg));

    let num_sid = numeric_session_id(session_seed);
    if !num_sid.is_empty() {
        inner_req.insert("sessionId".into(), Value::String(num_sid));
    }

    let envelope = json!({
        "project": project_id,
        "model": wire_model,
        "userAgent": "antigravity",
        "requestId": build_antigravity_request_id(session_seed, wire_model, content_count),
        "request": Value::Object(inner_req)
    });

    let encoded = serde_json::to_vec(&envelope)
        .map_err(|e| TesRouteError::Internal(format!("encode Antigravity envelope: {e}")))?;
    Ok(Bytes::from(encoded))
}

#[derive(Debug)]
pub struct AntigravitySseDecoder {
    buf: String,
    msg_id: String,
    model: String,
    started: bool,
    finished: bool,
    tool_call_index: usize,
    last_thought_sig: String,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
}

impl AntigravitySseDecoder {
    pub fn new(model: &str) -> Self {
        Self {
            buf: String::new(),
            msg_id: format!("chatcmpl-ag-{}", now_ms()),
            model: model.to_string(),
            started: false,
            finished: false,
            tool_call_index: 0,
            last_thought_sig: String::new(),
            prompt_tokens: 0,
            cached_tokens: 0,
            completion_tokens: 0,
        }
    }

    /// Non-cached input tokens (matching 9router accounting).
    pub fn uncached_prompt_tokens(&self) -> i64 {
        (self.prompt_tokens - self.cached_tokens).max(0)
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) -> Vec<Bytes> {
        self.buf.push_str(&String::from_utf8_lossy(chunk));
        let mut out = Vec::new();
        while let Some(nl_pos) = self.buf.find('\n') {
            let line = self.buf[..nl_pos].trim_end_matches('\r').to_string();
            self.buf.drain(..=nl_pos);
            if let Some(data) = line.strip_prefix("data:") {
                let payload = data.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                out.extend(self.translate_json_event(payload));
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<Bytes> {
        let mut out = Vec::new();
        let rem = std::mem::take(&mut self.buf);
        for line in rem.lines() {
            if let Some(data) = line.trim().strip_prefix("data:") {
                let payload = data.trim();
                if !payload.is_empty() && payload != "[DONE]" {
                    out.extend(self.translate_json_event(payload));
                }
            }
        }
        if !self.finished {
            self.finished = true;
            let stop_reason = if self.tool_call_index > 0 {
                "tool_calls"
            } else {
                "stop"
            };
            out.push(self.format_sse_chunk(&json!({
                "id": self.msg_id,
                "object": "chat.completion.chunk",
                "created": now_ms() / 1000,
                "model": self.model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": stop_reason
                }],
                "usage": {
                    "prompt_tokens": self.prompt_tokens,
                    "completion_tokens": self.completion_tokens,
                    "total_tokens": self.prompt_tokens + self.completion_tokens,
                    "prompt_tokens_details": {
                        "cached_tokens": self.cached_tokens
                    }
                }
            })));
        }
        out.push(Bytes::from_static(b"data: [DONE]\n\n"));
        out
    }

    fn format_sse_chunk(&self, val: &Value) -> Bytes {
        let s = serde_json::to_string(val).unwrap_or_else(|_| "{}".to_string());
        Bytes::from(format!("data: {s}\n\n"))
    }

    fn translate_json_event(&mut self, raw_json: &str) -> Vec<Bytes> {
        let Ok(outer) = serde_json::from_str::<Value>(raw_json) else {
            return Vec::new();
        };
        let inner = outer.get("response").unwrap_or(&outer);

        if let Some(um) = inner.get("usageMetadata") {
            if let Some(p) = um.get("promptTokenCount").and_then(Value::as_i64) {
                self.prompt_tokens = p;
            }
            let cand = um
                .get("candidatesTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let thoughts = um
                .get("thoughtsTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if cand > 0 || thoughts > 0 {
                self.completion_tokens = cand + thoughts;
            }
            if let Some(cached) = um
                .get("cachedContentTokenCount")
                .or_else(|| um.get("cachedContentToken"))
                .and_then(Value::as_i64)
            {
                self.cached_tokens = cached;
            }
        }

        let mut chunks = Vec::new();
        if !self.started {
            self.started = true;
            chunks.push(self.format_sse_chunk(&json!({
                "id": self.msg_id,
                "object": "chat.completion.chunk",
                "created": now_ms() / 1000,
                "model": self.model,
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant" },
                    "finish_reason": Value::Null
                }]
            })));
        }

        let Some(cand) = inner
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            return chunks;
        };

        if let Some(parts) = cand
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
        {
            for part in parts {
                if let Some(sig) = part
                    .get("thoughtSignature")
                    .or_else(|| part.get("thought_signature"))
                    .and_then(Value::as_str)
                    && !sig.is_empty()
                {
                    self.last_thought_sig = sig.to_string();
                }

                let is_thought = part.get("thought").and_then(Value::as_bool) == Some(true);
                if let Some(text) = part.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    let delta = if is_thought {
                        json!({ "reasoning_content": text })
                    } else {
                        json!({ "content": text })
                    };
                    chunks.push(self.format_sse_chunk(&json!({
                        "id": self.msg_id,
                        "object": "chat.completion.chunk",
                        "created": now_ms() / 1000,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": delta,
                            "finish_reason": Value::Null
                        }]
                    })));
                }

                if let Some(fc) = part.get("functionCall") {
                    let wire_name = fc.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let fn_name = uncloak_tool_name(wire_name);
                    let args_str = fc
                        .get("args")
                        .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "{}".into()))
                        .unwrap_or_else(|| "{}".into());

                    let base_id = fc
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{}_{}", now_ms(), self.tool_call_index));

                    let sig = part
                        .get("thoughtSignature")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&self.last_thought_sig);

                    let full_call_id = if sig.is_empty() {
                        base_id
                    } else {
                        format!("{base_id}__ts__{sig}")
                    };

                    let idx = self.tool_call_index;
                    self.tool_call_index += 1;

                    chunks.push(self.format_sse_chunk(&json!({
                        "id": self.msg_id,
                        "object": "chat.completion.chunk",
                        "created": now_ms() / 1000,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": idx,
                                    "id": full_call_id,
                                    "type": "function",
                                    "function": {
                                        "name": fn_name,
                                        "arguments": args_str
                                    }
                                }]
                            },
                            "finish_reason": Value::Null
                        }]
                    })));
                }
            }
        }

        if let Some(reason) = cand.get("finishReason").and_then(Value::as_str)
            && !reason.is_empty()
            && !self.finished
        {
            self.finished = true;
            let mut oa_stop = match reason.to_ascii_uppercase().as_str() {
                "MAX_TOKENS" => "length",
                "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" => "content_filter",
                _ => "stop",
            };
            if self.tool_call_index > 0 && oa_stop == "stop" {
                oa_stop = "tool_calls";
            }
            chunks.push(self.format_sse_chunk(&json!({
                "id": self.msg_id,
                "object": "chat.completion.chunk",
                "created": now_ms() / 1000,
                "model": self.model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": oa_stop
                }],
                "usage": {
                    "prompt_tokens": self.prompt_tokens,
                    "completion_tokens": self.completion_tokens,
                    "total_tokens": self.prompt_tokens + self.completion_tokens,
                    "prompt_tokens_details": {
                        "cached_tokens": self.cached_tokens
                    }
                }
            })));
        }

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_claude_5_5_and_gemini_4_wire_models() {
        assert_eq!(
            resolve_antigravity_upstream_model("claude-opus-5-5", None),
            "claude-opus-5-5-medium"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("ag/claude-opus-5-5", Some("low")),
            "claude-opus-5-5-low"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-opus-5-5", Some("medium")),
            "claude-opus-5-5-medium"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-opus-5-5", Some("high")),
            "claude-opus-5-5-high"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-opus-5-5", Some("xhigh")),
            "claude-opus-5-5-high"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-opus-5-5", Some("max")),
            "claude-opus-5-5-high"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-opus-5.5", Some("high")),
            "claude-opus-5-5-high"
        );

        assert_eq!(
            resolve_antigravity_upstream_model("claude-sonnet-5-5", None),
            "claude-sonnet-5-5-medium"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-sonnet-5-5", Some("low")),
            "claude-sonnet-5-5-low"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-sonnet-5-5", Some("medium")),
            "claude-sonnet-5-5-medium"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("claude-sonnet-5-5", Some("high")),
            "claude-sonnet-5-5-high"
        );

        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-neon", None),
            "gemini-4-neon-medium"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-neon", Some("low")),
            "gemini-4-neon-low"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-neon", Some("medium")),
            "gemini-4-neon-medium"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-neon", Some("high")),
            "gemini-4-neon-high"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-neon", Some("xhigh")),
            "gemini-4-neon-xhigh"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-neon", Some("max")),
            "gemini-4-neon-xhigh"
        );

        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-hydrogen", Some("xhigh")),
            "gemini-4-hydrogen-xhigh"
        );
        assert_eq!(
            resolve_antigravity_upstream_model("gemini-4-argon", Some("xhigh")),
            "gemini-4-argon-high"
        );

        assert_eq!(max_output_tokens_for_antigravity_wire("claude-opus-5-5-high"), 128_000);
        assert_eq!(max_output_tokens_for_antigravity_wire("gemini-4-neon-xhigh"), 262_144);
        assert_eq!(max_output_tokens_for_antigravity_wire("gemini-3.8-flash-high"), 65_536);
    }

    #[test]
    fn cleans_gemini_tool_schema_preventing_protobuf_errors() {
        // 1. Array type: `type: ["string", "null"]` -> `type: "string", nullable: true`
        let input_type_arr = json!({
            "type": ["string", "null"],
            "description": "optional string param"
        });
        let cleaned_type_arr = clean_json_schema(&input_type_arr);
        assert_eq!(cleaned_type_arr["type"], "string");
        assert_eq!(cleaned_type_arr["nullable"], true);
        assert_eq!(cleaned_type_arr["description"], "optional string param");

        // 2. Array items: `items: [{ "type": "string" }]` -> `items: { "type": "string" }`
        let input_items_arr = json!({
            "type": "array",
            "items": [{ "type": "string", "$schema": "http://json-schema.org/draft-07/schema#" }]
        });
        let cleaned_items_arr = clean_json_schema(&input_items_arr);
        assert_eq!(cleaned_items_arr["type"], "array");
        assert!(cleaned_items_arr["items"].is_object(), "items must be a single Schema object");
        assert_eq!(cleaned_items_arr["items"]["type"], "string");
        assert!(cleaned_items_arr["items"].get("$schema").is_none());

        // 3. Const field: `const: "exact_val"` -> `type: "string", enum: ["exact_val"]`
        let input_const = json!({
            "const": "prod_mode"
        });
        let cleaned_const = clean_json_schema(&input_const);
        assert_eq!(cleaned_const["type"], "string");
        assert_eq!(cleaned_const["enum"], json!(["prod_mode"]));
        assert!(cleaned_const.get("const").is_none(), "const must be eliminated");

        // 4. anyOf containing const branches (from Pydantic/Zod Literal unions)
        // e.g. `anyOf: [{ "const": "a" }, { "const": "b" }, { "const": "c" }]`
        let input_any_of_consts = json!({
            "anyOf": [
                { "const": "draft" },
                { "const": "published" },
                { "const": "archived" }
            ]
        });
        let cleaned_any_of = clean_json_schema(&input_any_of_consts);
        assert_eq!(cleaned_any_of["type"], "string");
        assert_eq!(cleaned_any_of["enum"], json!(["draft", "published", "archived"]));
        assert!(cleaned_any_of.get("const").is_none());

        // 5. Strips forbidden OpenAPI/JSON schema keys ($schema, additionalProperties, strict, default, etc.)
        let input_complex = json!({
            "type": "object",
            "$schema": "draft-07",
            "additionalProperties": false,
            "strict": true,
            "default": {},
            "properties": {
                "tags": {
                    "type": "array",
                    "items": [{ "type": ["string", "null"] }]
                },
                "status": {
                    "anyOf": [{ "const": "active" }, { "const": "inactive" }]
                }
            },
            "required": ["status"]
        });
        let cleaned_complex = clean_json_schema(&input_complex);
        assert_eq!(cleaned_complex["type"], "object");
        assert!(cleaned_complex.get("$schema").is_none());
        assert!(cleaned_complex.get("additionalProperties").is_none());
        assert!(cleaned_complex.get("strict").is_none());
        assert!(cleaned_complex.get("default").is_none());

        // Verify nested tags items
        let tags_items = &cleaned_complex["properties"]["tags"]["items"];
        assert!(tags_items.is_object());
        assert_eq!(tags_items["type"], "string");
        assert_eq!(tags_items["nullable"], true);

        // Verify nested status enum
        let status_prop = &cleaned_complex["properties"]["status"];
        assert_eq!(status_prop["type"], "string");
        assert_eq!(status_prop["enum"], json!(["active", "inactive"]));
    }

    #[test]
    fn build_antigravity_envelope_sanitizes_tool_declarations() {
        let chat_body = json!({
            "model": "gemini-3.8-flash",
            "messages": [
                { "role": "user", "content": "Hello" }
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "edit_file",
                        "description": "Edits a file",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "lines": {
                                    "type": "array",
                                    "items": [{ "type": ["string", "null"] }]
                                },
                                "mode": {
                                    "anyOf": [
                                        { "const": "replace" },
                                        { "const": "insert" }
                                    ]
                                }
                            },
                            "required": ["lines"]
                        }
                    }
                }
            ]
        });

        let bytes = build_antigravity_envelope(&chat_body, "test-proj", "gemini-3.8-flash", "seed-123")
            .expect("envelope build should succeed");
        let val: Value = serde_json::from_slice(&bytes).expect("valid json envelope");

        let decls = val["request"]["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("functionDeclarations array");

        let edit_tool = decls
            .iter()
            .find(|d| d["name"].as_str().map_or(false, |n| n.contains("edit_file")))
            .expect("edit_file tool found");

        let lines_items = &edit_tool["parameters"]["properties"]["lines"]["items"];
        assert!(lines_items.is_object(), "items must be a single Schema object");
        assert_eq!(lines_items["type"], "string");
        assert_eq!(lines_items["nullable"], true);

        let mode_prop = &edit_tool["parameters"]["properties"]["mode"];
        assert_eq!(mode_prop["type"], "string");
        assert_eq!(mode_prop["enum"], json!(["replace", "insert"]));
        assert!(mode_prop.get("const").is_none());
    }
}
