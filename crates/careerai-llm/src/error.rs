//! Error types for the LLM gateway.
//!
//! `Result<T>` aliases to `Result<T, LlmError>`. Only `thiserror` is used
//! here — no `anyhow` in library crates.

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("rate limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("schema: {0}")]
    Schema(String),

    #[error("upstream: {0}")]
    Upstream(String),

    #[error("timeout after {seconds}s")]
    Timeout { seconds: u64 },
}

pub type Result<T> = std::result::Result<T, LlmError>;
