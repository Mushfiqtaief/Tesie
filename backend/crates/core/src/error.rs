use thiserror::Error;

pub type Result<T> = std::result::Result<T, TesRouteError>;

#[derive(Debug, Error)]
pub enum TesRouteError {
    #[error("invalid request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("context length exceeded: {0}")]
    ContextLengthExceeded(String),

    #[error("all OpenCode exit IPs are currently rate limited (retry after {retry_after_secs}s)")]
    AllIpsRateLimited {
        retry_after_secs: u64,
        last_error: Option<String>,
    },

    #[error("{0}")]
    ModelQuotaExhausted(String),

    #[error("{message}")]
    ModelCoolingDown {
        message: String,
        retry_after_secs: u64,
    },

    #[error("api key quota exceeded: {0}")]
    ApiKeyQuotaExceeded(String),

    #[error("no credentials configured for provider {0}")]
    NoCredentials(String),

    #[error("upstream error ({status}): {message}")]
    Upstream { status: u16, message: String },

    #[error("proxy relay error: {0}")]
    ProxyRelay(String),

    #[error("all {attempts} upstream attempt(s) exhausted (last phase: {last_phase}): {message}")]
    UpstreamsExhausted {
        attempts: usize,
        last_phase: String,
        message: String,
    },

    #[error("storage error: {0}")]
    Store(String),

    #[error("internal error: {0}")]
    Internal(String),
}

/// Detects whether an upstream HTTP status code and response body signify that the model's
/// context length / token limit has been exceeded (e.g. OpenAI, Anthropic, Google, DeepSeek).
pub fn is_context_length_error(status: u16, body_or_err: &str) -> bool {
    if !matches!(status, 400 | 413 | 422) {
        return false;
    }
    let lower = body_or_err.to_ascii_lowercase();
    lower.contains("context_length_exceeded")
        || lower.contains("prompt is too long")
        || lower.contains("prompt is too large")
        || lower.contains("request is too large")
        || lower.contains("maximum context length")
        || lower.contains("exceeds the context window")
        || lower.contains("exceeded the context window")
        || lower.contains("context window exceeded")
        || lower.contains("context length exceeded")
        || lower.contains("context limit exceeded")
        || lower.contains("too many tokens")
        || lower.contains("input tokens exceed")
        || lower.contains("exceeds maximum allowed length")
        || lower.contains("token limit exceeded")
        || lower.contains("max_tokens is too large")
        || lower.contains("maximum input tokens")
        || lower.contains("input is too long")
        || (lower.contains("token") && lower.contains("exceed") && !lower.contains("rate"))
}

/// Unwraps or extracts the clean error message from upstream JSON payloads so downstream
/// AI coding clients (Claude Code, OpenCode CLI, Aider) can parse it without mangling.
pub fn extract_clean_context_message(body_or_err: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_or_err) {
        if let Some(msg) = val.pointer("/error/message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
        if let Some(msg) = val.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
        if let Some(msg) = val.get("error").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
    }
    body_or_err.trim().to_string()
}
