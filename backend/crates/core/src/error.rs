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

    #[error("storage error: {0}")]
    Store(String),

    #[error("internal error: {0}")]
    Internal(String),
}
