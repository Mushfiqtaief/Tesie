use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use chrono::DateTime;
use rand::RngCore;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

const BASE62_CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const SESSION_PREFIX_MASK: u64 = (1u64 << 48) - 1;
const MAX_SESSION_LENGTH: usize = 256;
const SESSION_TIMESTAMP_WINDOW_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

static SESSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ses_[0-9a-f]{12}[0-9A-Za-z]{14}$").unwrap());
static DESCENDING_TIME_PART_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{9}f[c-f][0-9a-f]$").unwrap());
static GIT_COMMIT_PROJECT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{40}$").unwrap());
static OPENCODE_V2_UA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^opencode/[a-z0-9-]+/(\d+)\.\d+\.\d+/[a-z0-9-]+$").unwrap());
static CLAUDE_SESSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_session_([0-9A-Fa-f-]{36})$").unwrap());

static BOOT_NONCE: LazyLock<[u8; 16]> = LazyLock::new(|| {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
});

static SESSION_CLOCK: Mutex<(u64, u32)> = Mutex::new((0, 0));
static UPSTREAM_CLOCK_OFFSET_MS: AtomicI64 = AtomicI64::new(0);

pub const OPENCODE_V2_UA: &str = "opencode/latest/2.1.1/cli";
pub const OPENCODE_CLIENT: &str = "cli";

/// Calibrates our session/prompt clock against `opencode.ai`'s HTTP `Date` response header.
/// Free-tier risk checks reject `ses_` timestamps skewed significantly from upstream UTC time
/// with synthetic `429 FreeUsageLimitError`.
pub fn observe_upstream_date_header(date_header: &str) {
    if let Ok(parsed) = DateTime::parse_from_rfc2822(date_header.trim()) {
        let upstream_ms = parsed.timestamp_millis();
        let local_ms = tesie_store::now_ms();
        let offset = upstream_ms.saturating_sub(local_ms);
        UPSTREAM_CLOCK_OFFSET_MS.store(offset, Ordering::Relaxed);
    }
}

pub fn upstream_now_ms() -> i64 {
    let local_ms = tesie_store::now_ms();
    let offset = UPSTREAM_CLOCK_OFFSET_MS.load(Ordering::Relaxed);
    local_ms.saturating_add(offset)
}

pub fn is_valid_opencode_session(value: &str) -> bool {
    let trimmed = value.trim();
    SESSION_RE.is_match(trimmed) && DESCENDING_TIME_PART_RE.is_match(&trimmed[4..16])
}

/// Deterministic `opencode2api`-compatible `CanonicalSessionID`:
/// Preserves genuine official `Identifier.descending()` `ses_` IDs verbatim, and re-encodes
/// any other conversation signal with `BOOT_NONCE` + today's UTC midnight minus a deterministic
/// offset within 30 days (`counter = 1`, ending in `ffe`) + 14 Base62 chars.
pub fn canonical_session_id(signal: &str) -> String {
    let trimmed = signal.trim();
    if is_valid_opencode_session(trimmed) {
        return trimmed.to_string();
    }
    let mut hasher = Sha256::new();
    hasher.update(b"ses\0");
    hasher.update(*BOOT_NONCE);
    hasher.update([0u8]);
    hasher.update(trimmed.as_bytes());
    let sum = hasher.finalize();

    let offset = (u64::from_be_bytes([0, 0, 0, 0, sum[0], sum[1], sum[2], sum[3]]))
        % SESSION_TIMESTAMP_WINDOW_MS;
    let now = upstream_now_ms().max(DAY_MS);
    let midnight_utc = (now / DAY_MS) * DAY_MS;
    let base_ms = (midnight_utc as u64).saturating_sub(offset);
    let time_part = descending_time_hex(base_ms, 1);
    let random_part = base62_from_10_bytes(&sum[6..16], 14);
    format!("ses_{time_part}{random_part}")
}

fn base62_from_10_bytes(bytes: &[u8], width: usize) -> String {
    let mut n: u128 = 0;
    for &b in bytes {
        n = (n << 8) | (b as u128);
    }
    let mut out = vec![b'0'; width];
    for i in (0..width).rev() {
        let rem = (n % 62) as usize;
        n /= 62;
        out[i] = BASE62_CHARS[rem];
    }
    String::from_utf8(out).unwrap_or_default()
}

pub fn is_valid_v2_project_id(value: &str) -> bool {
    GIT_COMMIT_PROJECT_RE.is_match(value.trim())
}

/// Generates a fresh 40-char lowercase hex git-commit-style project ID.
/// OpenCode's free tier ledges anonymous usage per `(project_id, egress_ip)` and rejects
/// `prj_*` IDs on v2 with `403 FreeTierError`, so fresh 40-hex per request avoids shared
/// project-bucket exhaustion.
pub fn random_project_id() -> String {
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn has_valid_opencode_v2_ua(ua: &str) -> bool {
    if let Some(caps) = OPENCODE_V2_UA_RE.captures(ua.trim())
        && let Some(m) = caps.get(1)
        && let Ok(major) = m.as_str().parse::<u32>()
    {
        return major >= 2;
    }
    false
}

fn descending_time_hex(time_ms: u64, counter: u32) -> String {
    let raw = time_ms.wrapping_mul(0x1000).wrapping_add(counter as u64);
    let masked = (!raw) & SESSION_PREFIX_MASK;
    format!("{masked:012x}")
}

fn random_base62(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
        .into_iter()
        .map(|b| BASE62_CHARS[(b as usize) % BASE62_CHARS.len()] as char)
        .collect()
}

pub fn generate_session_id(now_ms: Option<u64>) -> String {
    let now = now_ms.unwrap_or_else(|| upstream_now_ms().max(0) as u64);
    let (ts, ctr) = {
        let mut guard = SESSION_CLOCK.lock().unwrap();
        if now > guard.0 {
            guard.0 = now;
            guard.1 = 1;
        } else {
            guard.1 += 1;
            // Upstream decodes the counter and rejects >=100 with 403 FreeTierError; keep <= 16.
            if guard.1 > 16 {
                guard.0 = guard.0.saturating_add(1);
                guard.1 = 1;
            }
        }
        *guard
    };
    format!("ses_{}{}", descending_time_hex(ts, ctr), random_base62(14))
}

pub fn generate_request_id(now_ms: Option<u64>) -> String {
    let now = now_ms.unwrap_or_else(|| upstream_now_ms().max(0) as u64);
    let val = (now.wrapping_mul(0x1000).wrapping_add(1)) & SESSION_PREFIX_MASK;
    format!("msg_{val:012x}{}", random_base62(14))
}

pub fn header_ci<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

pub fn normalize_session_header(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_SESSION_LENGTH {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn extract_native_opencode_session(headers: &[(String, String)]) -> Option<String> {
    let raw = header_ci(headers, "x-opencode-session")?;
    let norm = normalize_session_header(raw)?;
    if is_valid_opencode_session(&norm) {
        Some(norm)
    } else {
        None
    }
}

fn claude_code_session_id(user_id: &str) -> Option<String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(caps) = CLAUDE_SESSION_RE.captures(trimmed)
        && let Some(m) = caps.get(1)
    {
        return Some(m.as_str().to_string());
    }
    if let Ok(val) = serde_json::from_str::<Value>(trimmed)
        && let Some(sid) = val.get("session_id").and_then(Value::as_str)
        && !sid.trim().is_empty()
    {
        return Some(sid.trim().to_string());
    }
    None
}

pub fn extract_parent_session(headers: &[(String, String)], body: &Value) -> Option<String> {
    let raw = header_ci(headers, "x-opencode-parent-session-id")
        .or_else(|| header_ci(headers, "x-parent-session-id"))
        .or_else(|| {
            body.get("metadata")
                .and_then(Value::as_object)
                .and_then(|m| m.get("parent_session_id"))
                .and_then(Value::as_str)
        })?;
    let norm = normalize_session_header(raw)?;
    Some(canonical_session_id(&norm))
}

pub fn extract_child_session(headers: &[(String, String)]) -> Option<String> {
    let raw = header_ci(headers, "x-opencode-session-id")?;
    let norm = normalize_session_header(raw)?;
    Some(canonical_session_id(&norm))
}

pub fn extract_conversation_hint(headers: &[(String, String)], body: &Value) -> Option<String> {
    for hname in [
        "x-opencode-session",
        "x-opencode-session-id",
        "x-session-affinity",
        "x-session-id",
        "session_id",
        "conversation-id",
        "conversation_id",
    ] {
        if let Some(raw) = header_ci(headers, hname)
            && let Some(norm) = normalize_session_header(raw)
        {
            return Some(format!("hdr:{norm}"));
        }
    }

    if let Some(obj) = body.as_object() {
        for field in ["session_id", "conversation_id", "prompt_cache_key"] {
            if let Some(s) = obj.get(field).and_then(Value::as_str)
                && !s.trim().is_empty()
            {
                return Some(format!("{field}:{}", s.trim()));
            }
        }
        if let Some(meta) = obj.get("metadata").and_then(Value::as_object) {
            if let Some(sid) = meta.get("session_id").and_then(Value::as_str)
                && !sid.trim().is_empty()
            {
                return Some(format!("meta_ses:{}", sid.trim()));
            }
            if let Some(uid) = meta.get("user_id").and_then(Value::as_str) {
                if let Some(cc_ses) = claude_code_session_id(uid) {
                    return Some(format!("cc_ses:{cc_ses}"));
                }
                if !uid.trim().is_empty() {
                    return Some(format!("user:{}", uid.trim()));
                }
            }
        }

        // Conversation seed matching opencode2api: system/instructions prelude + first user turn
        let mut prelude: Vec<&Value> = Vec::new();
        for sys_field in ["system", "instructions"] {
            if let Some(v) = obj.get(sys_field)
                && !v.is_null()
            {
                prelude.push(v);
            }
        }
        if let Some(inp_str) = obj.get("input").and_then(Value::as_str)
            && !inp_str.trim().is_empty()
        {
            let mut hasher = Sha256::new();
            for p in &prelude {
                hasher.update(p.to_string().as_bytes());
            }
            hasher.update(inp_str.as_bytes());
            return Some(format!("seed:{}", hex::encode(&hasher.finalize()[..16])));
        }
        let items = obj
            .get("messages")
            .and_then(Value::as_array)
            .or_else(|| obj.get("input").and_then(Value::as_array));
        if let Some(arr) = items {
            for item in arr {
                match item.get("role").and_then(Value::as_str) {
                    Some("system" | "developer") => {
                        if let Some(c) = item.get("content") {
                            prelude.push(c);
                        }
                    }
                    Some("user") => {
                        if let Some(c) = item.get("content") {
                            let mut hasher = Sha256::new();
                            for p in &prelude {
                                hasher.update(p.to_string().as_bytes());
                            }
                            hasher.update(c.to_string().as_bytes());
                            return Some(format!(
                                "seed:{}",
                                hex::encode(&hasher.finalize()[..16])
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(prev) = obj.get("previous_response_id").and_then(Value::as_str)
            && !prev.trim().is_empty()
        {
            return Some(format!("prev_resp:{}", prev.trim()));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_v2_session_and_project_ids() {
        for _ in 0..25 {
            let sid = generate_session_id(Some(1_760_000_000_000));
            assert!(is_valid_opencode_session(&sid), "invalid session: {sid}");
            let pid = random_project_id();
            assert!(is_valid_v2_project_id(&pid), "invalid project id: {pid}");
        }
        assert!(has_valid_opencode_v2_ua(OPENCODE_V2_UA));
        assert!(!has_valid_opencode_v2_ua("opencode/1.18.31"));
    }
}
