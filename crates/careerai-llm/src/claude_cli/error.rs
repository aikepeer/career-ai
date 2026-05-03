//! Error types for the `claude` CLI subprocess driver.
//!
//! Mapped into [`LlmError`] at the trait boundary via `From`.

use crate::error::LlmError;

/// Errors specific to the `claude` CLI subprocess driver.
#[derive(Debug, thiserror::Error)]
pub enum ClaudeCliError {
    #[error("`claude` binary not found on PATH (install Claude Code or set CAREERAI_CLAUDE_BIN)")]
    NotInstalled,

    #[error("`claude` binary at {path} is not usable: {reason}")]
    BinaryUnusable { path: String, reason: String },

    #[error("claude CLI session is not authenticated; run `claude login` (or `/login` in claude)")]
    AuthExpired,

    #[error("claude CLI rate-limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("claude CLI transport error: {0}")]
    Transport(String),

    #[error("claude CLI returned malformed JSON: {0}")]
    ParseJson(String),

    #[error("claude CLI timed out after {seconds}s")]
    Timeout { seconds: u64 },
}

impl From<ClaudeCliError> for LlmError {
    fn from(value: ClaudeCliError) -> Self {
        match value {
            ClaudeCliError::NotInstalled | ClaudeCliError::AuthExpired => {
                LlmError::Upstream(value.to_string())
            }
            ClaudeCliError::BinaryUnusable { .. } => LlmError::Upstream(value.to_string()),
            ClaudeCliError::RateLimited {
                retry_after_seconds,
            } => LlmError::RateLimited {
                retry_after_seconds,
            },
            ClaudeCliError::Transport(msg) => LlmError::Upstream(format!("claude-cli: {msg}")),
            ClaudeCliError::ParseJson(msg) => LlmError::Schema(format!("claude-cli: {msg}")),
            ClaudeCliError::Timeout { seconds } => LlmError::Timeout { seconds },
        }
    }
}
