use std::path::{Path, PathBuf};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tesie_store::Store;

use crate::{err_resp, AdminState};

fn user_home_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("USERPROFILE").filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    PathBuf::from(".")
}

fn opencode_config_path() -> PathBuf {
    user_home_dir()
        .join(".config")
        .join("opencode")
        .join("opencode.json")
}

fn claude_settings_path() -> PathBuf {
    user_home_dir().join(".claude").join("settings.json")
}

fn codex_config_path() -> PathBuf {
    user_home_dir().join(".codex").join("config.toml")
}

fn binary_exists_on_system(bin_name: &str) -> bool {
    let mut search_dirs: Vec<PathBuf> = Vec::new();
    if let Some(path_os) = std::env::var_os("PATH") {
        search_dirs.extend(std::env::split_paths(&path_os));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        search_dirs.push(PathBuf::from(appdata).join("npm"));
    }
    let home = user_home_dir();
    search_dirs.push(home.join(".opencode").join("bin"));
    search_dirs.push(home.join(".local").join("bin"));
    search_dirs.push(home.join(".cargo").join("bin"));

    #[cfg(windows)]
    let exts = ["", ".exe", ".cmd", ".bat", ".ps1"];
    #[cfg(not(windows))]
    let exts = [""];

    for dir in search_dirs {
        for ext in exts {
            let candidate = dir.join(format!("{bin_name}{ext}"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

fn strip_jsonc_trailing_commas(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    let mut escaped = false;

    while i < chars.len() {
        let ch = chars[i];
        if in_str {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_str = true;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn read_json_object_lenient(path: &Path) -> Option<Map<String, Value>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let cleaned = strip_jsonc_trailing_commas(&raw);
    match serde_json::from_str::<Value>(&cleaned).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn normalize_v1_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

async fn resolve_cli_api_key(store: &Store, explicit_key: Option<&str>) -> String {
    if let Some(k) = explicit_key.map(str::trim).filter(|s| !s.is_empty()) {
        return k.to_string();
    }
    if let Ok(keys) = store.keys().list_keys().await {
        if let Some(found) = keys
            .iter()
            .find(|k| k.enabled && k.raw_key.as_deref().is_some_and(|r| !r.is_empty()))
            && let Some(raw) = &found.raw_key
        {
            return raw.clone();
        }
    }
    "sk-tesie-local".to_string()
}

// ============================================================================
// Status Summary
// ============================================================================

pub fn inspect_opencode_at(path: &Path, bin_found: bool) -> Value {
    let installed = bin_found || path.exists();
    let cfg = read_json_object_lenient(path);
    let tr_provider = cfg
        .as_ref()
        .and_then(|c| c.get("provider"))
        .and_then(Value::as_object)
        .and_then(|p| p.get("tesie").or_else(|| p.get("tesroute")))
        .and_then(Value::as_object);

    let models: Vec<String> = tr_provider
        .and_then(|tp| tp.get("models"))
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let base_url = tr_provider
        .and_then(|tp| tp.get("options"))
        .and_then(Value::as_object)
        .and_then(|o| o.get("baseURL"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let raw_active = cfg
        .as_ref()
        .and_then(|c| c.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let active_model = raw_active
        .strip_prefix("tesie/")
        .or_else(|| raw_active.strip_prefix("tesroute/"))
        .map(str::to_string);

    let subagent_model = cfg
        .as_ref()
        .and_then(|c| c.get("agent"))
        .and_then(Value::as_object)
        .and_then(|a| a.get("explorer"))
        .and_then(Value::as_object)
        .and_then(|e| e.get("model"))
        .and_then(Value::as_str)
        .and_then(|m| m.strip_prefix("tesie/").or_else(|| m.strip_prefix("tesroute/")))
        .map(str::to_string);

    json!({
        "installed": installed,
        "configured": tr_provider.is_some(),
        "config_path": path.to_string_lossy(),
        "base_url": base_url,
        "models": models,
        "active_model": active_model,
        "subagent_model": subagent_model
    })
}

pub fn inspect_claude_at(path: &Path, bin_found: bool) -> Value {
    let installed = bin_found || path.exists();
    let cfg = read_json_object_lenient(path);
    let env_obj = cfg
        .as_ref()
        .and_then(|c| c.get("env"))
        .and_then(Value::as_object);

    let base_url = env_obj
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let get_env_str = |k: &str| -> Option<String> {
        env_obj
            .and_then(|e| e.get(k))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    json!({
        "installed": installed,
        "configured": base_url.is_some(),
        "config_path": path.to_string_lossy(),
        "base_url": base_url,
        "default_model": get_env_str("ANTHROPIC_MODEL"),
        "opus_model": get_env_str("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        "sonnet_model": get_env_str("ANTHROPIC_DEFAULT_SONNET_MODEL"),
        "haiku_model": get_env_str("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        "subagent_model": get_env_str("CLAUDE_CODE_SUBAGENT_MODEL")
    })
}

pub fn inspect_codex_at(path: &Path, bin_found: bool) -> Value {
    let installed = bin_found || path.exists();
    let raw = std::fs::read_to_string(path).ok();
    let configured = raw.as_deref().is_some_and(|s| {
        s.contains("[model_providers.tesie]")
            || s.contains("model_provider = \"tesie\"")
            || s.contains("[model_providers.tesroute]")
            || s.contains("model_provider = \"tesroute\"")
    });

    let mut active_model = None;
    let mut base_url = None;
    if let Some(text) = &raw {
        for line in text.lines() {
            let trimmed = line.trim();
            if active_model.is_none()
                && let Some(rest) = trimmed.strip_prefix("model =")
            {
                active_model = Some(rest.trim().trim_matches('"').to_string());
            }
            if let Some(rest) = trimmed.strip_prefix("base_url =") {
                base_url = Some(rest.trim().trim_matches('"').to_string());
            }
        }
    }

    json!({
        "installed": installed,
        "configured": configured,
        "config_path": path.to_string_lossy(),
        "base_url": base_url,
        "active_model": active_model
    })
}

pub async fn get_all_cli_statuses() -> Json<Value> {
    let oc_path = opencode_config_path();
    let cl_path = claude_settings_path();
    let cx_path = codex_config_path();

    Json(json!({
        "opencode": inspect_opencode_at(&oc_path, binary_exists_on_system("opencode")),
        "claude": inspect_claude_at(&cl_path, binary_exists_on_system("claude")),
        "codex": inspect_codex_at(&cx_path, binary_exists_on_system("codex")),
    }))
}

// ============================================================================
// OpenCode Config Merge & Reset
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ApplyOpenCodeInput {
    pub base_url: String,
    pub api_key: Option<String>,
    pub models: Vec<String>,
    pub active_model: Option<String>,
    pub subagent_model: Option<String>,
}

pub fn apply_opencode_to_path(
    path: &Path,
    base_url: &str,
    api_key: &str,
    models: &[String],
    active_model: Option<&str>,
    subagent_model: Option<&str>,
) -> std::result::Result<(), String> {
    let clean_models: Vec<String> = models
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if clean_models.is_empty() {
        return Err("Select at least one model to expose in OpenCode".into());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }

    let mut root = read_json_object_lenient(path).unwrap_or_default();
    if !root.contains_key("$schema") {
        root.insert(
            "$schema".into(),
            Value::String("https://opencode.ai/config.json".into()),
        );
    }

    let normalized_url = normalize_v1_base_url(base_url);
    let providers_entry = root
        .entry("provider")
        .or_insert_with(|| Value::Object(Map::new()));
    if !providers_entry.is_object() {
        *providers_entry = Value::Object(Map::new());
    }
    let providers_obj = providers_entry.as_object_mut().unwrap();

    let mut models_map = Map::new();
    for m in &clean_models {
        models_map.insert(
            m.clone(),
            json!({
                "name": m,
                "modalities": {
                    "input": ["text", "image"],
                    "output": ["text"]
                }
            }),
        );
    }

    providers_obj.remove("tesroute");
    providers_obj.insert(
        "tesie".into(),
        json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "Tesie",
            "options": {
                "baseURL": normalized_url,
                "apiKey": api_key
            },
            "models": models_map
        }),
    );

    let chosen_active = active_model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&clean_models[0]);
    root.insert(
        "model".into(),
        Value::String(format!("tesie/{chosen_active}")),
    );

    let chosen_sub = subagent_model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(chosen_active);
    let agent_entry = root
        .entry("agent")
        .or_insert_with(|| Value::Object(Map::new()));
    if !agent_entry.is_object() {
        *agent_entry = Value::Object(Map::new());
    }
    if let Some(agent_obj) = agent_entry.as_object_mut() {
        agent_obj.insert(
            "explorer".into(),
            json!({
                "description": "Fast explorer subagent routed via Tesie",
                "mode": "subagent",
                "model": format!("tesie/{chosen_sub}")
            }),
        );
    }

    let serialized =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    std::fs::write(path, serialized).map_err(|e| format!("write opencode.json: {e}"))?;
    Ok(())
}

pub fn reset_opencode_at_path(path: &Path, model_to_remove: Option<&str>) -> std::result::Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let Some(mut root) = read_json_object_lenient(path) else {
        return Ok(());
    };

    if let Some(target_model) = model_to_remove.map(str::trim).filter(|s| !s.is_empty()) {
        let mut remove_provider = false;
        let mut fallback_model: Option<String> = None;
        let active_key = if root
            .get("provider")
            .and_then(Value::as_object)
            .is_some_and(|p| p.contains_key("tesie"))
        {
            "tesie"
        } else {
            "tesroute"
        };
        if let Some(tr) = root
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .and_then(|p| p.get_mut(active_key))
            .and_then(Value::as_object_mut)
            && let Some(models_obj) = tr.get_mut("models").and_then(Value::as_object_mut)
        {
            models_obj.remove(target_model);
            if models_obj.is_empty() {
                remove_provider = true;
            } else {
                fallback_model = models_obj.keys().next().cloned();
            }
        }
        if remove_provider {
            if let Some(p) = root.get_mut("provider").and_then(Value::as_object_mut) {
                p.remove("tesie");
                p.remove("tesroute");
            }
            if root
                .get("model")
                .and_then(Value::as_str)
                .is_some_and(|m| m.starts_with("tesie/") || m.starts_with("tesroute/"))
            {
                root.remove("model");
            }
        } else if root.get("model").and_then(Value::as_str).is_some_and(|m| {
            m == format!("tesie/{target_model}") || m == format!("tesroute/{target_model}")
        }) && let Some(fb) = fallback_model
        {
            root.insert("model".into(), Value::String(format!("{active_key}/{fb}")));
        }
    } else {
        if let Some(p) = root.get_mut("provider").and_then(Value::as_object_mut) {
            p.remove("tesie");
            p.remove("tesroute");
        }
        if root
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|m| m.starts_with("tesie/") || m.starts_with("tesroute/"))
        {
            root.remove("model");
        }
        if let Some(agent_obj) = root.get_mut("agent").and_then(Value::as_object_mut) {
            let should_remove = agent_obj
                .get("explorer")
                .and_then(Value::as_object)
                .and_then(|e| e.get("model"))
                .and_then(Value::as_str)
                .is_some_and(|m| m.starts_with("tesie/") || m.starts_with("tesroute/"));
            if should_remove {
                agent_obj.remove("explorer");
            }
            if agent_obj.is_empty() {
                root.remove("agent");
            }
        }
    }

    let serialized =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    std::fs::write(path, serialized).map_err(|e| format!("write opencode.json: {e}"))?;
    Ok(())
}

pub async fn get_opencode_settings() -> Json<Value> {
    let path = opencode_config_path();
    Json(inspect_opencode_at(&path, binary_exists_on_system("opencode")))
}

pub async fn apply_opencode_settings(
    State(state): State<AdminState>,
    Json(input): Json<ApplyOpenCodeInput>,
) -> std::result::Result<Json<Value>, Response> {
    let key = resolve_cli_api_key(&state.store, input.api_key.as_deref()).await;
    let path = opencode_config_path();
    apply_opencode_to_path(
        &path,
        &input.base_url,
        &key,
        &input.models,
        input.active_model.as_deref(),
        input.subagent_model.as_deref(),
    )
    .map_err(|msg| {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
    })?;

    Ok(Json(json!({
        "success": true,
        "status": inspect_opencode_at(&path, true)
    })))
}

#[derive(Deserialize, Default)]
pub struct RemoveModelQuery {
    pub model: Option<String>,
}

pub async fn reset_opencode_settings(
    Query(q): Query<RemoveModelQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let path = opencode_config_path();
    reset_opencode_at_path(&path, q.model.as_deref()).map_err(|msg| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": msg }))).into_response()
    })?;
    Ok(Json(json!({
        "success": true,
        "status": inspect_opencode_at(&path, binary_exists_on_system("opencode"))
    })))
}

// ============================================================================
// Claude Code Config Merge & Reset
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ApplyClaudeInput {
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub opus_model: Option<String>,
    pub sonnet_model: Option<String>,
    pub haiku_model: Option<String>,
    pub subagent_model: Option<String>,
}

const CLAUDE_MANAGED_ENV_KEYS: [&str; 7] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

pub fn apply_claude_to_path(
    path: &Path,
    input: &ApplyClaudeInput,
    resolved_key: &str,
) -> std::result::Result<(), String> {
    if input.base_url.trim().is_empty() {
        return Err("base_url is required".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create .claude dir: {e}"))?;
    }

    let mut root = read_json_object_lenient(path).unwrap_or_default();
    root.insert("hasCompletedOnboarding".into(), Value::Bool(true));

    let env_entry = root
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    if !env_entry.is_object() {
        *env_entry = Value::Object(Map::new());
    }
    let env_obj = env_entry.as_object_mut().unwrap();

    // Anthropic SDK appends `/v1/messages` to `ANTHROPIC_BASE_URL`, and our gateway also
    // exposes `/v1/v1/messages` so either root or `/v1` works cleanly; writing the clean
    // origin URL keeps Claude Code telemetry neat.
    let clean_origin = input
        .base_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    env_obj.insert("ANTHROPIC_BASE_URL".into(), Value::String(clean_origin));
    env_obj.insert(
        "ANTHROPIC_AUTH_TOKEN".into(),
        Value::String(resolved_key.to_string()),
    );

    let set_or_clear = |env: &mut Map<String, Value>, key: &str, val: &Option<String>| {
        if let Some(v) = val.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            env.insert(key.into(), Value::String(v.to_string()));
        } else {
            env.remove(key);
        }
    };

    set_or_clear(env_obj, "ANTHROPIC_MODEL", &input.default_model);
    set_or_clear(env_obj, "ANTHROPIC_DEFAULT_OPUS_MODEL", &input.opus_model);
    set_or_clear(
        env_obj,
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        &input.sonnet_model,
    );
    set_or_clear(env_obj, "ANTHROPIC_DEFAULT_HAIKU_MODEL", &input.haiku_model);
    set_or_clear(
        env_obj,
        "CLAUDE_CODE_SUBAGENT_MODEL",
        &input.subagent_model,
    );

    let serialized =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    std::fs::write(path, serialized).map_err(|e| format!("write claude settings.json: {e}"))?;
    Ok(())
}

pub fn reset_claude_at_path(path: &Path) -> std::result::Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let Some(mut root) = read_json_object_lenient(path) else {
        return Ok(());
    };
    if let Some(env_obj) = root.get_mut("env").and_then(Value::as_object_mut) {
        for k in CLAUDE_MANAGED_ENV_KEYS {
            env_obj.remove(k);
        }
        if env_obj.is_empty() {
            root.remove("env");
        }
    }
    let serialized =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    std::fs::write(path, serialized).map_err(|e| format!("write claude settings.json: {e}"))?;
    Ok(())
}

pub async fn get_claude_settings() -> Json<Value> {
    let path = claude_settings_path();
    Json(inspect_claude_at(&path, binary_exists_on_system("claude")))
}

pub async fn apply_claude_settings(
    State(state): State<AdminState>,
    Json(input): Json<ApplyClaudeInput>,
) -> std::result::Result<Json<Value>, Response> {
    let key = resolve_cli_api_key(&state.store, input.api_key.as_deref()).await;
    let path = claude_settings_path();
    apply_claude_to_path(&path, &input, &key).map_err(|msg| {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
    })?;
    Ok(Json(json!({
        "success": true,
        "status": inspect_claude_at(&path, true)
    })))
}

pub async fn reset_claude_settings() -> std::result::Result<Json<Value>, Response> {
    let path = claude_settings_path();
    reset_claude_at_path(&path).map_err(|msg| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": msg }))).into_response()
    })?;
    Ok(Json(json!({
        "success": true,
        "status": inspect_claude_at(&path, binary_exists_on_system("claude"))
    })))
}

// ============================================================================
// Codex CLI Config Merge & Reset
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ApplyCodexInput {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub subagent_model: Option<String>,
}

const CODEX_BLOCK_START: &str = "# --- TESIE MANAGED START ---";
const CODEX_BLOCK_END: &str = "# --- TESIE MANAGED END ---";
const LEGACY_CODEX_START: &str = "# --- TESROUTE MANAGED START ---";
const LEGACY_CODEX_END: &str = "# --- TESROUTE MANAGED END ---";

fn strip_codex_managed_block(existing: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == CODEX_BLOCK_START || trimmed == LEGACY_CODEX_START {
            inside = true;
            continue;
        }
        if trimmed == CODEX_BLOCK_END || trimmed == LEGACY_CODEX_END {
            inside = false;
            continue;
        }
        if !inside {
            if trimmed.starts_with("model =") || trimmed.starts_with("model_provider =") {
                continue;
            }
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

pub fn apply_codex_to_path(
    path: &Path,
    input: &ApplyCodexInput,
    resolved_key: &str,
) -> std::result::Result<(), String> {
    let model = input.model.trim();
    if input.base_url.trim().is_empty() || model.is_empty() {
        return Err("base_url and model are required".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create .codex dir: {e}"))?;
    }

    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let cleaned = strip_codex_managed_block(&existing);
    let normalized_v1 = normalize_v1_base_url(&input.base_url);
    let sub_model = input
        .subagent_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(model);

    let managed = format!(
        r#"model = "{model}"
model_provider = "tesie"

{CODEX_BLOCK_START}
[model_providers.tesie]
name = "Tesie"
base_url = "{normalized_v1}"
wire_api = "responses"
http_headers = {{ Authorization = "Bearer {resolved_key}" }}

[agents]
default_subagent_model = "{sub_model}"
{CODEX_BLOCK_END}
"#
    );

    let final_toml = if cleaned.is_empty() {
        managed
    } else {
        format!("{managed}\n{cleaned}\n")
    };
    std::fs::write(path, final_toml).map_err(|e| format!("write codex config.toml: {e}"))?;
    Ok(())
}

pub fn reset_codex_at_path(path: &Path) -> std::result::Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let cleaned = strip_codex_managed_block(&existing);
    std::fs::write(path, if cleaned.is_empty() { String::new() } else { format!("{cleaned}\n") })
        .map_err(|e| format!("write codex config.toml: {e}"))?;
    Ok(())
}

pub async fn get_codex_settings() -> Json<Value> {
    let path = codex_config_path();
    Json(inspect_codex_at(&path, binary_exists_on_system("codex")))
}

pub async fn apply_codex_settings(
    State(state): State<AdminState>,
    Json(input): Json<ApplyCodexInput>,
) -> std::result::Result<Json<Value>, Response> {
    let key = resolve_cli_api_key(&state.store, input.api_key.as_deref()).await;
    let path = codex_config_path();
    apply_codex_to_path(&path, &input, &key).map_err(|msg| {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
    })?;
    let _ = &state;
    Ok(Json(json!({
        "success": true,
        "status": inspect_codex_at(&path, true)
    })))
}

pub async fn reset_codex_settings() -> std::result::Result<Json<Value>, Response> {
    let path = codex_config_path();
    reset_codex_at_path(&path).map_err(|msg| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": msg }))).into_response()
    })?;
    Ok(Json(json!({
        "success": true,
        "status": inspect_codex_at(&path, binary_exists_on_system("codex"))
    })))
}

#[allow(dead_code)]
fn _keep_err_resp_ref() {
    let _ = err_resp;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_apply_and_reset_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tesroute_oc_test_{}", uuid::Uuid::new_v4()));
        let path = dir.join("opencode.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"theme": "dark",}"#).unwrap();

        apply_opencode_to_path(
            &path,
            "http://127.0.0.1:20129",
            "sk-tr-test",
            &[
                "oc/glm-5.3-flash-free".to_string(),
                "smart-combo".to_string(),
            ],
            Some("smart-combo"),
            Some("oc/glm-5.3-flash-free"),
        )
        .unwrap();

        let status = inspect_opencode_at(&path, false);
        assert_eq!(status["configured"], true);
        assert_eq!(status["active_model"], "smart-combo");
        assert_eq!(status["subagent_model"], "oc/glm-5.3-flash-free");
        assert_eq!(status["base_url"], "http://127.0.0.1:20129/v1");

        reset_opencode_at_path(&path, None).unwrap();
        let after = inspect_opencode_at(&path, false);
        assert_eq!(after["configured"], false);
        let raw_json = read_json_object_lenient(&path).unwrap();
        assert_eq!(raw_json.get("theme").and_then(Value::as_str), Some("dark"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claude_apply_and_reset_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tesroute_cl_test_{}", uuid::Uuid::new_v4()));
        let path = dir.join("settings.json");
        apply_claude_to_path(
            &path,
            &ApplyClaudeInput {
                base_url: "http://127.0.0.1:20129/v1".into(),
                api_key: None,
                default_model: Some("oc/glm-5.3-flash-free".into()),
                opus_model: Some("smart-combo".into()),
                sonnet_model: Some("oc/glm-5.3-flash-free".into()),
                haiku_model: None,
                subagent_model: None,
            },
            "sk-tr-abc",
        )
        .unwrap();

        let st = inspect_claude_at(&path, false);
        assert_eq!(st["configured"], true);
        assert_eq!(st["base_url"], "http://127.0.0.1:20129");
        assert_eq!(st["opus_model"], "smart-combo");

        reset_claude_at_path(&path).unwrap();
        let st2 = inspect_claude_at(&path, false);
        assert_eq!(st2["configured"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
