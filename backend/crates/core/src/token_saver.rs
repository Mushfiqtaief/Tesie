use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::WireFormat;

pub const RAW_CAP_BYTES: usize = 10 * 1024 * 1024;
const MIN_COMPRESS_CHARS: usize = 320;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenSaverCategories {
    #[serde(default = "default_true")]
    pub git: bool,
    #[serde(default = "default_true")]
    pub build: bool,
    #[serde(default = "default_true")]
    pub test: bool,
    #[serde(default = "default_true")]
    pub package: bool,
    #[serde(default = "default_true")]
    pub docker: bool,
    #[serde(default = "default_true")]
    pub system: bool,
}

impl Default for TokenSaverCategories {
    fn default() -> Self {
        Self {
            git: true,
            build: true,
            test: true,
            package: true,
            docker: true,
            system: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenSaverConfig {
    #[serde(default = "default_true")]
    pub rtk_enabled: bool,
    #[serde(default = "default_intensity")]
    pub rtk_intensity: String,
    #[serde(default = "default_max_lines")]
    pub rtk_max_lines: usize,
    #[serde(default = "default_true")]
    pub rtk_deduplicate: bool,
    #[serde(default)]
    pub rtk_categories: TokenSaverCategories,
    /// `"off"` | `"caveman"` | `"ponytail"` | `"adhd"` | `"custom"`
    #[serde(default = "default_off")]
    pub style_mode: String,
    /// `"lite"` | `"full"` | `"ultra"`
    #[serde(default = "default_full")]
    pub style_level: String,
    #[serde(default)]
    pub custom_prompt: String,
}

impl Default for TokenSaverConfig {
    fn default() -> Self {
        Self {
            rtk_enabled: true,
            rtk_intensity: "standard".to_string(),
            rtk_max_lines: 250,
            rtk_deduplicate: true,
            rtk_categories: TokenSaverCategories::default(),
            style_mode: "off".to_string(),
            style_level: "full".to_string(),
            custom_prompt: String::new(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_intensity() -> String {
    "standard".to_string()
}
fn default_max_lines() -> usize {
    250
}
fn default_off() -> String {
    "off".to_string()
}
fn default_full() -> String {
    "full".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtkCompressResult {
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub saved_tokens: usize,
    pub saved_pct: f64,
    pub text: String,
    pub detected_category: String,
    pub techniques_used: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RequestSavingsReport {
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub saved_tokens: usize,
    pub compressed_blocks: usize,
    pub primary_category: String,
    pub style_applied: bool,
}

pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        (text.len() + 3) / 4
    }
}

pub fn is_protected_tool(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "read"
            | "read_file"
            | "view_file"
            | "grep"
            | "edit"
            | "edit_file"
            | "write"
            | "write_file"
            | "patch"
            | "apply_patch"
            | "anchor"
            | "glob"
            | "view"
            | "list_dir"
    )
}

pub fn is_error_tool_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    [
        "Error:",
        "Exception:",
        "Traceback (most recent call last):",
        "error[E",
        "panicked at",
        "FAILED",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn strip_ansi_sequences(s: &str) -> (String, bool) {
    if !s.contains('\x1b') {
        return (s.to_string(), false);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut stripped = false;
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            stripped = true;
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    let _ = chars.next();
                    while let Some(seq_c) = chars.next() {
                        if ('@'..='~').contains(&seq_c) {
                            break;
                        }
                    }
                    continue;
                } else if next == ']' {
                    let _ = chars.next();
                    while let Some(osc_c) = chars.next() {
                        if osc_c == '\x07' || osc_c == '\n' {
                            break;
                        }
                    }
                    continue;
                }
            }
            continue;
        }
        out.push(c);
    }
    (out, stripped)
}

fn is_valid_json_payload(s: &str) -> bool {
    let trimmed = s.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}'))
        && !(trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        return false;
    }
    serde_json::from_str::<Value>(trimmed).is_ok()
}

pub fn detect_category(text: &str, command_hint: Option<&str>) -> &'static str {
    if let Some(cmd) = command_hint {
        let lower = cmd.to_ascii_lowercase();
        if lower.contains("git ") || lower == "git" || lower.contains("gh ") {
            return "git";
        }
        if lower.contains("docker ") || lower.contains("kubectl ") || lower.contains("helm ") {
            return "docker";
        }
        if lower.contains("cargo test")
            || lower.contains("pytest")
            || lower.contains("vitest")
            || lower.contains("jest")
            || lower.contains("go test")
            || lower.contains("bun test")
            || lower.contains("npm test")
        {
            return "test";
        }
        if lower.contains("cargo ")
            || lower.contains("tsc")
            || lower.contains("vite")
            || lower.contains("webpack")
            || lower.contains("eslint")
            || lower.contains("biome")
            || lower.contains("go build")
            || lower.contains("make")
            || lower.contains("gradle")
            || lower.contains("dotnet ")
        {
            return "build";
        }
        if lower.contains("npm ")
            || lower.contains("bun ")
            || lower.contains("pnpm ")
            || lower.contains("yarn ")
            || lower.contains("pip ")
            || lower.contains("uv ")
            || lower.contains("poetry ")
            || lower.contains("composer ")
        {
            return "package";
        }
        if lower.contains("ls")
            || lower.contains("dir")
            || lower.contains("find")
            || lower.contains("ps ")
            || lower.contains("df ")
            || lower.contains("du ")
            || lower.contains("curl ")
            || lower.contains("wget ")
            || lower.contains("systemctl")
        {
            return "system";
        }
    }

    let trimmed = text.trim_start();
    if trimmed.starts_with("diff --git")
        || trimmed.starts_with("--- a/")
        || trimmed.contains("On branch ")
        || trimmed.contains("Changes not staged for commit:")
        || trimmed.contains("Changes to be committed:")
    {
        return "git";
    }
    if trimmed.contains("Compiling ")
        || trimmed.contains("Downloading crates")
        || trimmed.contains("error TS")
        || trimmed.contains("vite v")
        || trimmed.contains("transformed.")
    {
        return "build";
    }
    if trimmed.contains("test result:")
        || (trimmed.contains("running ") && trimmed.contains(" tests"))
        || trimmed.contains(" passed in ")
        || trimmed.contains("PASS ")
    {
        return "test";
    }
    if trimmed.contains("Requirement already satisfied:")
        || trimmed.contains("added ") && trimmed.contains(" packages in ")
        || trimmed.contains("npm warn")
    {
        return "package";
    }
    if trimmed.contains("CONTAINER ID")
        || trimmed.contains("Pulling fs layer")
        || trimmed.contains("Verifying Checksum")
    {
        return "docker";
    }
    if trimmed.contains("USER       PID") || trimmed.starts_with("total ") {
        return "system";
    }
    "generic"
}

fn category_enabled(cat: &str, cfg: &TokenSaverConfig) -> bool {
    match cat {
        "git" => cfg.rtk_categories.git,
        "build" => cfg.rtk_categories.build,
        "test" => cfg.rtk_categories.test,
        "package" => cfg.rtk_categories.package,
        "docker" => cfg.rtk_categories.docker,
        "system" => cfg.rtk_categories.system,
        _ => true,
    }
}

fn compress_git_diff_hunks(lines: Vec<&str>) -> (Vec<String>, bool) {
    if !lines.iter().any(|l| l.starts_with("diff --git") || l.starts_with("@@ ")) {
        return (lines.into_iter().map(str::to_string).collect(), false);
    }
    const MAX_HUNK_LINES: usize = 80;
    let mut out = Vec::with_capacity(lines.len());
    let mut in_hunk = false;
    let mut hunk_count = 0usize;
    let mut skipped_in_hunk = 0usize;
    let mut changed = false;

    for line in lines {
        let is_boundary = line.starts_with("diff --git ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("@@ ");
        if is_boundary {
            if skipped_in_hunk > 0 {
                out.push(format!(
                    "... [{skipped_in_hunk} diff lines omitted by Tesie RTK]"
                ));
                skipped_in_hunk = 0;
            }
            in_hunk = line.starts_with("@@ ");
            hunk_count = 0;
            out.push(line.to_string());
            continue;
        }
        if in_hunk {
            hunk_count += 1;
            if hunk_count > MAX_HUNK_LINES {
                skipped_in_hunk += 1;
                changed = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if skipped_in_hunk > 0 {
        out.push(format!(
            "... [{skipped_in_hunk} diff lines omitted by Tesie RTK]"
        ));
    }
    (out, changed)
}

fn compress_build_lines(lines: Vec<&str>) -> (Vec<String>, bool) {
    if lines.len() <= 24 {
        return (lines.into_iter().map(str::to_string).collect(), false);
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut skipped = 0usize;
    for line in lines {
        let t = line.trim_start();
        let is_noise = t.starts_with("Compiling ")
            || t.starts_with("Downloading ")
            || t.starts_with("Downloaded ")
            || t.starts_with("Checking ")
            || t.starts_with("Fetch ")
            || t.starts_with("  Downloading ");
        if is_noise {
            skipped += 1;
        } else {
            if skipped > 0 {
                out.push(format!("... [{skipped} build progress lines collapsed]"));
                skipped = 0;
            }
            out.push(line.to_string());
        }
    }
    if skipped > 0 {
        out.push(format!("... [{skipped} build progress lines collapsed]"));
    }
    let changed = out.len() < lines_len_hint(&out) || out.iter().any(|l| l.contains("collapsed]"));
    (out, changed)
}

fn lines_len_hint(_out: &[String]) -> usize {
    0
}

fn compress_package_lines(lines: Vec<&str>) -> (Vec<String>, bool) {
    if lines.len() <= 20 {
        return (lines.into_iter().map(str::to_string).collect(), false);
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut skipped = 0usize;
    for line in lines {
        let t = line.trim_start();
        let is_noise = t.starts_with("Requirement already satisfied:")
            || t.starts_with("npm http fetch ")
            || t.starts_with("npm timing ")
            || t.starts_with("Get:")
            || t.starts_with("Fetched ");
        if is_noise {
            skipped += 1;
        } else {
            if skipped > 0 {
                out.push(format!("... [{skipped} package resolution lines collapsed]"));
                skipped = 0;
            }
            out.push(line.to_string());
        }
    }
    if skipped > 0 {
        out.push(format!("... [{skipped} package resolution lines collapsed]"));
    }
    let changed = out.iter().any(|l| l.contains("collapsed]"));
    (out, changed)
}

fn compress_test_lines(lines: Vec<&str>) -> (Vec<String>, bool) {
    if lines.len() <= 40 {
        return (lines.into_iter().map(str::to_string).collect(), false);
    }
    let has_failures = lines.iter().any(|l| {
        let low = l.to_ascii_lowercase();
        low.contains("failed") || low.contains("panicked") || low.contains("assertion")
    });
    if has_failures {
        return (lines.into_iter().map(str::to_string).collect(), false);
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut passing = 0usize;
    for line in lines {
        let t = line.trim();
        let is_passing = (t.starts_with("test ") && t.ends_with("... ok"))
            || t.starts_with("✓ ")
            || t.starts_with("PASS ");
        if is_passing {
            passing += 1;
        } else {
            if passing > 0 {
                out.push(format!("... [{passing} passing test lines collapsed]"));
                passing = 0;
            }
            out.push(line.to_string());
        }
    }
    if passing > 0 {
        out.push(format!("... [{passing} passing test lines collapsed]"));
    }
    let changed = out.iter().any(|l| l.contains("collapsed]"));
    (out, changed)
}

fn compress_docker_lines(lines: Vec<&str>) -> (Vec<String>, bool) {
    if lines.len() <= 20 {
        return (lines.into_iter().map(str::to_string).collect(), false);
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut skipped = 0usize;
    for line in lines {
        let t = line.trim();
        let is_noise = t.ends_with("Pulling fs layer")
            || t.ends_with("Waiting")
            || t.ends_with("Verifying Checksum")
            || t.ends_with("Download complete")
            || t.contains(": Extracting [");
        if is_noise {
            skipped += 1;
        } else {
            if skipped > 0 {
                out.push(format!("... [{skipped} container layer lines collapsed]"));
                skipped = 0;
            }
            out.push(line.to_string());
        }
    }
    if skipped > 0 {
        out.push(format!("... [{skipped} container layer lines collapsed]"));
    }
    let changed = out.iter().any(|l| l.contains("collapsed]"));
    (out, changed)
}

fn deduplicate_consecutive_lines(lines: Vec<String>) -> (Vec<String>, bool) {
    if lines.len() < 3 {
        return (lines, false);
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut changed = false;
    let mut i = 0;
    while i < lines.len() {
        let cur_trimmed = lines[i].trim();
        let mut j = i + 1;
        while j < lines.len()
            && (lines[j] == lines[i] || (!cur_trimmed.is_empty() && lines[j].trim() == cur_trimmed))
        {
            j += 1;
        }
        let count = j - i;
        out.push(lines[i].clone());
        if count >= 3 {
            out.push(format!("... (repeated {} times)", count - 1));
            changed = true;
            i = j;
        } else if count == 2 {
            out.push(lines[i + 1].clone());
            i = j;
        } else {
            i += 1;
        }
    }
    (out, changed)
}

fn resolve_max_lines(cfg: &TokenSaverConfig) -> usize {
    let base = cfg.rtk_max_lines.max(40);
    match cfg.rtk_intensity.trim().to_ascii_lowercase().as_str() {
        "minimal" => base.saturating_mul(2),
        "aggressive" => (base / 2).max(60),
        _ => base,
    }
}

fn smart_truncate_lines(lines: Vec<String>, max_lines: usize) -> (Vec<String>, bool) {
    if lines.len() <= max_lines {
        return (lines, false);
    }
    let head_len = ((max_lines as f64) * 0.65).round() as usize;
    let tail_len = max_lines.saturating_sub(head_len).max(10);
    if head_len + tail_len >= lines.len() {
        return (lines, false);
    }
    let omitted = lines.len() - head_len - tail_len;
    let mut out = Vec::with_capacity(head_len + tail_len + 1);
    out.extend_from_slice(&lines[..head_len]);
    out.push(format!(
        "... [Tesie RTK: omitted {omitted} middle lines] ..."
    ));
    out.extend_from_slice(&lines[lines.len() - tail_len..]);
    (out, true)
}

pub fn compress_text_detailed(
    text: &str,
    tool_name: Option<&str>,
    command_hint: Option<&str>,
    cfg: &TokenSaverConfig,
) -> RtkCompressResult {
    let orig_tokens = estimate_tokens(text);
    let detected = detect_category(text, command_hint).to_string();

    if !cfg.rtk_enabled
        || text.len() < MIN_COMPRESS_CHARS
        || text.len() > RAW_CAP_BYTES
        || tool_name.is_some_and(is_protected_tool)
        || is_error_tool_text(text)
        || is_valid_json_payload(text)
        || !category_enabled(&detected, cfg)
    {
        return RtkCompressResult {
            original_tokens: orig_tokens,
            compressed_tokens: orig_tokens,
            saved_tokens: 0,
            saved_pct: 0.0,
            text: text.to_string(),
            detected_category: detected,
            techniques_used: Vec::new(),
        };
    }

    let mut techniques = Vec::new();
    let (ansi_clean, stripped_ansi) = strip_ansi_sequences(text);
    if stripped_ansi {
        techniques.push("ansi_strip".to_string());
    }

    let normalized = ansi_clean.replace("\r\n", "\n");
    let raw_lines: Vec<&str> = normalized.split('\n').collect();

    let (mut lines, cat_changed): (Vec<String>, bool) = match detected.as_str() {
        "git" => compress_git_diff_hunks(raw_lines),
        "build" => compress_build_lines(raw_lines),
        "package" => compress_package_lines(raw_lines),
        "test" => compress_test_lines(raw_lines),
        "docker" => compress_docker_lines(raw_lines),
        _ => (raw_lines.into_iter().map(str::to_string).collect(), false),
    };
    if cat_changed {
        techniques.push(format!("{detected}_filter"));
    }

    if cfg.rtk_deduplicate {
        let (deduped, dedup_changed) = deduplicate_consecutive_lines(lines);
        lines = deduped;
        if dedup_changed {
            techniques.push("deduplicate_lines".to_string());
        }
    }

    let max_lines = resolve_max_lines(cfg);
    let (truncated, trunc_changed) = smart_truncate_lines(lines, max_lines);
    lines = truncated;
    if trunc_changed {
        techniques.push("smart_truncate".to_string());
    }

    let candidate = lines.join("\n");
    if candidate.len() >= text.len() {
        return RtkCompressResult {
            original_tokens: orig_tokens,
            compressed_tokens: orig_tokens,
            saved_tokens: 0,
            saved_pct: 0.0,
            text: text.to_string(),
            detected_category: detected,
            techniques_used: Vec::new(),
        };
    }

    let comp_tokens = estimate_tokens(&candidate);
    let saved_tokens = orig_tokens.saturating_sub(comp_tokens);
    let saved_pct = if orig_tokens > 0 && saved_tokens > 0 {
        ((saved_tokens as f64 / orig_tokens as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    RtkCompressResult {
        original_tokens: orig_tokens,
        compressed_tokens: comp_tokens,
        saved_tokens,
        saved_pct,
        text: candidate,
        detected_category: detected,
        techniques_used: techniques,
    }
}

pub fn style_prompt_for(mode: &str, level: &str, custom: &str) -> Option<String> {
    let clean_mode = mode.trim().to_ascii_lowercase();
    let clean_level = level.trim().to_ascii_lowercase();
    const CODE_GUARD: &str = "Code blocks, file paths, symbol names, CLI commands, URLs, and error strings: keep exact.";

    match clean_mode.as_str() {
        "caveman" => {
            let body = match clean_level.as_str() {
                "lite" => "Respond concisely. Drop conversational filler, greetings, and closing pleasantries while keeping normal grammar.",
                "ultra" => "Telegraphic maximum compression. Fragments OK, strip articles/hedging/fluff, bullet facts tightly, zero preamble or outro.",
                _ => "Respond tersely like a senior engineer. Drop filler, articles when clear, and pleasantries. Fragments OK; lead directly with facts or fixes.",
            };
            Some(format!("[Tesie Token Saver: Caveman ({clean_level})] {body} {CODE_GUARD}"))
        }
        "ponytail" => {
            let body = match clean_level.as_str() {
                "lite" => "Prefer the simplest pragmatic solution. Reuse stdlib/existing helpers and point out any simpler alternative briefly.",
                "ultra" => "Strict YAGNI minimalism: avoid new abstractions, reuse existing code/stdlib first, prefer deleting code over adding indirection.",
                _ => "Prefer minimal native/stdlib solutions first. Do not introduce extra abstractions or dependencies when a small direct helper suffices.",
            };
            Some(format!("[Tesie Token Saver: Ponytail ({clean_level})] {body} {CODE_GUARD}"))
        }
        "adhd" => {
            let body = match clean_level.as_str() {
                "lite" => "Lead with the immediate action or answer, keep lists short (<=5 items), bold key values, no preamble or closer.",
                _ => "Action-first formatting: (1) lead with the concrete command, fix, or answer; (2) use numbered bounded steps; (3) cap lists at 5 bullets; (4) no preamble or closing fluff.",
            };
            Some(format!("[Tesie Token Saver: ADHD ({clean_level})] {body} {CODE_GUARD}"))
        }
        "custom" => {
            let trimmed = custom.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn has_bypass_header(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, v)| {
        (k.eq_ignore_ascii_case("x-tesie-token-saver")
            || k.eq_ignore_ascii_case("x-9router-token-saver"))
            && matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0")
    })
}

fn extract_command_from_args(args: Option<&Value>) -> Option<String> {
    let val = args?;
    if let Some(obj) = val.as_object() {
        for key in ["command", "CommandLine", "cmd", "script"] {
            if let Some(s) = obj.get(key).and_then(Value::as_str)
                && !s.trim().is_empty()
            {
                return Some(s.trim().to_string());
            }
        }
    } else if let Some(raw) = val.as_str()
        && let Ok(parsed) = serde_json::from_str::<Value>(raw)
    {
        return extract_command_from_args(Some(&parsed));
    }
    None
}

fn index_tool_calls(items: &[Value]) -> HashMap<String, (String, Option<String>)> {
    let mut map = HashMap::new();
    for item in items {
        if let Some(tc_list) = item.get("tool_calls").and_then(Value::as_array) {
            for tc in tc_list {
                if let Some(id) = tc.get("id").and_then(Value::as_str)
                    && let Some(func) = tc.get("function")
                {
                    let name = func
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let cmd = extract_command_from_args(func.get("arguments"));
                    map.insert(id.to_string(), (name, cmd));
                }
            }
        }
        if item.get("type").and_then(Value::as_str) == Some("function_call")
            && let Some(call_id) = item.get("call_id").and_then(Value::as_str)
        {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let cmd = extract_command_from_args(item.get("arguments"));
            map.insert(call_id.to_string(), (name, cmd));
        }
        if let Some(blocks) = item.get("content").and_then(Value::as_array) {
            for blk in blocks {
                if blk.get("type").and_then(Value::as_str) == Some("tool_use")
                    && let Some(id) = blk.get("id").and_then(Value::as_str)
                {
                    let name = blk
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let cmd = extract_command_from_args(blk.get("input"));
                    map.insert(id.to_string(), (name, cmd));
                }
            }
        }
    }
    map
}

fn record_block_metrics(
    report: &mut RequestSavingsReport,
    res: &RtkCompressResult,
    category_counts: &mut HashMap<String, usize>,
) {
    if res.saved_tokens > 0 {
        report.original_tokens += res.original_tokens;
        report.compressed_tokens += res.compressed_tokens;
        report.saved_tokens += res.saved_tokens;
        report.compressed_blocks += 1;
        *category_counts
            .entry(res.detected_category.clone())
            .or_insert(0) += res.saved_tokens;
    }
}

fn compress_tool_outputs_in_body(
    body: &mut Value,
    format: WireFormat,
    cfg: &TokenSaverConfig,
    report: &mut RequestSavingsReport,
) {
    if !cfg.rtk_enabled {
        return;
    }
    let mut cat_savings: HashMap<String, usize> = HashMap::new();

    match format {
        WireFormat::OpenAiChat => {
            if let Some(msgs) = body.get_mut("messages").and_then(Value::as_array_mut) {
                let tool_map = index_tool_calls(msgs);
                for msg in msgs.iter_mut() {
                    if msg.get("role").and_then(Value::as_str) != Some("tool") {
                        continue;
                    }
                    let tool_call_id = msg
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let (tool_name, cmd) = tool_map
                        .get(tool_call_id)
                        .map(|(n, c)| (Some(n.as_str()), c.as_deref()))
                        .unwrap_or((None, None));
                    if let Some(content) = msg.get("content").and_then(Value::as_str) {
                        let res = compress_text_detailed(content, tool_name, cmd, cfg);
                        if res.saved_tokens > 0 {
                            record_block_metrics(report, &res, &mut cat_savings);
                            if let Some(obj) = msg.as_object_mut() {
                                obj.insert("content".into(), Value::String(res.text));
                            }
                        }
                    }
                }
            }
        }
        WireFormat::OpenAiResponses => {
            if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
                let tool_map = index_tool_calls(input);
                for item in input.iter_mut() {
                    if item.get("type").and_then(Value::as_str) != Some("function_call_output") {
                        continue;
                    }
                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let (tool_name, cmd) = tool_map
                        .get(call_id)
                        .map(|(n, c)| (Some(n.as_str()), c.as_deref()))
                        .unwrap_or((None, None));
                    if let Some(output) = item.get("output").and_then(Value::as_str) {
                        let res = compress_text_detailed(output, tool_name, cmd, cfg);
                        if res.saved_tokens > 0 {
                            record_block_metrics(report, &res, &mut cat_savings);
                            if let Some(obj) = item.as_object_mut() {
                                obj.insert("output".into(), Value::String(res.text));
                            }
                        }
                    }
                }
            }
        }
        WireFormat::AnthropicMessages => {
            if let Some(msgs) = body.get_mut("messages").and_then(Value::as_array_mut) {
                let tool_map = index_tool_calls(msgs);
                for msg in msgs.iter_mut() {
                    let Some(blocks) = msg.get_mut("content").and_then(Value::as_array_mut) else {
                        continue;
                    };
                    for blk in blocks.iter_mut() {
                        if blk.get("type").and_then(Value::as_str) != Some("tool_result") {
                            continue;
                        }
                        if blk.get("is_error").and_then(Value::as_bool) == Some(true) {
                            continue;
                        }
                        let tool_use_id = blk
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let (tool_name, cmd) = tool_map
                            .get(tool_use_id)
                            .map(|(n, c)| (Some(n.as_str()), c.as_deref()))
                            .unwrap_or((None, None));

                        if let Some(content_str) = blk.get("content").and_then(Value::as_str) {
                            let res = compress_text_detailed(content_str, tool_name, cmd, cfg);
                            if res.saved_tokens > 0 {
                                record_block_metrics(report, &res, &mut cat_savings);
                                if let Some(obj) = blk.as_object_mut() {
                                    obj.insert("content".into(), Value::String(res.text));
                                }
                            }
                        } else if let Some(inner_arr) =
                            blk.get_mut("content").and_then(Value::as_array_mut)
                        {
                            for inner in inner_arr.iter_mut() {
                                if inner.get("type").and_then(Value::as_str) == Some("text")
                                    && let Some(text) = inner.get("text").and_then(Value::as_str)
                                {
                                    let res = compress_text_detailed(text, tool_name, cmd, cfg);
                                    if res.saved_tokens > 0 {
                                        record_block_metrics(report, &res, &mut cat_savings);
                                        if let Some(obj) = inner.as_object_mut() {
                                            obj.insert("text".into(), Value::String(res.text));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some((top_cat, _)) = cat_savings.into_iter().max_by_key(|(_, v)| *v) {
        report.primary_category = top_cat;
    }
}

fn inject_style_prompt(body: &mut Value, format: WireFormat, directive: &str) -> bool {
    match format {
        WireFormat::OpenAiChat => {
            let Some(obj) = body.as_object_mut() else {
                return false;
            };
            let Some(msgs) = obj.get_mut("messages").and_then(Value::as_array_mut) else {
                return false;
            };
            for m in msgs.iter_mut() {
                let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
                if role == "system" || role == "developer" {
                    if let Some(content) = m.get("content").and_then(Value::as_str) {
                        if content.contains(directive) {
                            return false;
                        }
                        let merged = if content.trim().is_empty() {
                            directive.to_string()
                        } else {
                            format!("{content}\n\n{directive}")
                        };
                        if let Some(m_obj) = m.as_object_mut() {
                            m_obj.insert("content".into(), Value::String(merged));
                        }
                        return true;
                    }
                }
            }
            msgs.insert(0, json!({ "role": "system", "content": directive }));
            true
        }
        WireFormat::OpenAiResponses => {
            let Some(obj) = body.as_object_mut() else {
                return false;
            };
            if let Some(existing) = obj.get("instructions").and_then(Value::as_str) {
                if existing.contains(directive) {
                    return false;
                }
                let merged = if existing.trim().is_empty() {
                    directive.to_string()
                } else {
                    format!("{existing}\n\n{directive}")
                };
                obj.insert("instructions".into(), Value::String(merged));
                return true;
            }
            obj.insert("instructions".into(), Value::String(directive.to_string()));
            true
        }
        WireFormat::AnthropicMessages => {
            let Some(obj) = body.as_object_mut() else {
                return false;
            };
            match obj.get_mut("system") {
                Some(Value::String(s)) => {
                    if s.contains(directive) {
                        return false;
                    }
                    let merged = if s.trim().is_empty() {
                        directive.to_string()
                    } else {
                        format!("{s}\n\n{directive}")
                    };
                    obj.insert("system".into(), Value::String(merged));
                    true
                }
                Some(Value::Array(arr)) => {
                    let already = arr.iter().any(|b| {
                        b.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|t| t.contains(directive))
                    });
                    if already {
                        return false;
                    }
                    arr.push(json!({ "type": "text", "text": directive }));
                    true
                }
                _ => {
                    obj.insert("system".into(), Value::String(directive.to_string()));
                    true
                }
            }
        }
    }
}

pub fn apply_token_saver_to_request(
    body: &mut Value,
    format: WireFormat,
    headers: &[(String, String)],
    cfg: &TokenSaverConfig,
) -> RequestSavingsReport {
    let mut report = RequestSavingsReport {
        primary_category: "generic".to_string(),
        ..Default::default()
    };
    if has_bypass_header(headers) {
        return report;
    }

    compress_tool_outputs_in_body(body, format, cfg, &mut report);

    if let Some(directive) = style_prompt_for(&cfg.style_mode, &cfg.style_level, &cfg.custom_prompt)
    {
        report.style_applied = inject_style_prompt(body, format, &directive);
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compresses_cargo_build_and_is_deterministic() {
        let mut lines = Vec::new();
        for i in 0..80 {
            lines.push(format!("   Compiling crate-{i} v0.1.{i}"));
        }
        lines.push("warning: unused variable `x`".to_string());
        lines.push("    Finished `dev` profile [unoptimized] target(s) in 3.2s".to_string());
        let sample = lines.join("\n");

        let cfg = TokenSaverConfig::default();
        let first = compress_text_detailed(&sample, Some("shell"), Some("cargo build"), &cfg);
        assert!(first.saved_tokens > 0);
        assert_eq!(first.detected_category, "build");
        assert!(first.text.contains("warning: unused variable"));
        assert!(first.text.contains("Finished"));

        let second = compress_text_detailed(&first.text, Some("shell"), Some("cargo build"), &cfg);
        assert_eq!(second.text, first.text);
    }

    #[test]
    fn protects_read_tool_json_errors_and_bypass_header() {
        let cfg = TokenSaverConfig::default();
        let long_code = (0..100).map(|i| format!("let line_{i} = {i};")).collect::<Vec<_>>().join("\n");
        let read_res = compress_text_detailed(&long_code, Some("Read"), None, &cfg);
        assert_eq!(read_res.saved_tokens, 0);
        assert_eq!(read_res.text, long_code);

        let err_text = format!("Error: something failed\n{long_code}");
        let err_res = compress_text_detailed(&err_text, Some("shell"), None, &cfg);
        assert_eq!(err_res.saved_tokens, 0);

        let big_json = format!(r#"{{"items": [{}], "note": "{}"}}"#, "1,2,3", "x".repeat(400));
        let json_res = compress_text_detailed(&big_json, Some("shell"), None, &cfg);
        assert_eq!(json_res.saved_tokens, 0);

        let mut body = json!({
            "model": "oc/test",
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "c1", "function": { "name": "shell", "arguments": "{\"command\":\"cargo build\"}" } }] },
                { "role": "tool", "tool_call_id": "c1", "content": format!("{}\nFinished dev", "   Compiling a\n".repeat(45)) }
            ]
        });
        let report = apply_token_saver_to_request(
            &mut body,
            WireFormat::OpenAiChat,
            &[("X-Tesie-Token-Saver".into(), "off".into())],
            &cfg,
        );
        assert_eq!(report.saved_tokens, 0);
    }

    #[test]
    fn style_injection_is_idempotent_across_wire_formats() {
        let mut cfg = TokenSaverConfig::default();
        cfg.style_mode = "caveman".to_string();
        cfg.style_level = "full".to_string();

        let mut chat = json!({
            "model": "oc/test",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let r1 = apply_token_saver_to_request(&mut chat, WireFormat::OpenAiChat, &[], &cfg);
        assert!(r1.style_applied);
        let r2 = apply_token_saver_to_request(&mut chat, WireFormat::OpenAiChat, &[], &cfg);
        assert!(!r2.style_applied);

        let mut anthropic = json!({
            "model": "ag/claude-sonnet-4.6",
            "system": "Be helpful.",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let a1 = apply_token_saver_to_request(&mut anthropic, WireFormat::AnthropicMessages, &[], &cfg);
        assert!(a1.style_applied);
        let a2 = apply_token_saver_to_request(&mut anthropic, WireFormat::AnthropicMessages, &[], &cfg);
        assert!(!a2.style_applied);
    }
}
