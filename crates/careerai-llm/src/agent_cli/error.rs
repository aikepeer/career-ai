//! Error types for the `claude` CLI subprocess driver.
//!
//! Mapped into [`LlmError`] at the trait boundary via `From`.

use crate::error::LlmError;

use super::response::ClaudeCliResult;

/// Errors specific to CLI and agent subprocess drivers.
#[derive(Debug, thiserror::Error)]
pub enum AgentCliError {
    #[error("CLI binary not found on PATH (install agent CLI or set binary override)")]
    NotInstalled,

    #[error("CLI binary at {path} is not usable: {reason}")]
    BinaryUnusable { path: String, reason: String },

    #[error("CLI session is not authenticated; run `login` command")]
    AuthExpired,

    #[error("CLI rate-limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("CLI transport error: {0}")]
    Transport(String),

    #[error("CLI returned malformed JSON: {0}")]
    ParseJson(String),

    #[error("CLI timed out after {seconds}s")]
    Timeout { seconds: u64 },
}

/// Backwards-compatible type alias for `AgentCliError`.
pub type ClaudeCliError = AgentCliError;

impl AgentCliError {
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Timeout { .. } | Self::Transport(_)
        )
    }
}

impl From<AgentCliError> for LlmError {
    fn from(value: AgentCliError) -> Self {
        match value {
            AgentCliError::NotInstalled | AgentCliError::AuthExpired => {
                LlmError::Upstream(value.to_string())
            }
            AgentCliError::BinaryUnusable { .. } => LlmError::Upstream(value.to_string()),
            AgentCliError::RateLimited {
                retry_after_seconds,
            } => LlmError::RateLimited {
                retry_after_seconds,
            },
            AgentCliError::Transport(msg) => LlmError::Upstream(format!("cli: {msg}")),
            AgentCliError::ParseJson(msg) => {
                LlmError::Schema(format!("claude-cli returned malformed JSON: {msg}"))
            }
            AgentCliError::Timeout { seconds } => LlmError::Timeout { seconds },
        }
    }
}

/// Best-effort classifier for stderr text when stdout had no JSON.
pub(crate) fn classify_failure_stderr(stderr: &str) -> ClaudeCliError {
    let lc = stderr.to_lowercase();
    if lc.contains("not logged in") || lc.contains("/login") {
        ClaudeCliError::AuthExpired
    } else if lc.contains("rate") && lc.contains("limit") {
        ClaudeCliError::RateLimited {
            retry_after_seconds: 60,
        }
    } else if stderr.is_empty() {
        ClaudeCliError::Transport("empty stdout/stderr from claude".into())
    } else {
        let snippet: String = stderr.chars().take(256).collect();
        ClaudeCliError::Transport(snippet)
    }
}

/// Classify a parsed-but-errorful JSON payload.
pub(crate) fn classify_error_payload(parsed: &ClaudeCliResult) -> ClaudeCliError {
    // claude carries the message in `result`; agy carries it in `error`.
    let msg = parsed
        .result
        .clone()
        .or_else(|| parsed.error.clone())
        .unwrap_or_default();
    let lc = msg.to_lowercase();

    if let Some(status) = parsed.api_error_status {
        match status {
            401 | 403 => return ClaudeCliError::AuthExpired,
            429 => {
                return ClaudeCliError::RateLimited {
                    retry_after_seconds: 60,
                };
            }
            _ => {}
        }
    }

    if lc.contains("not logged in") || lc.contains("/login") || lc.contains("invalid api key") {
        return ClaudeCliError::AuthExpired;
    }
    if lc.contains("rate") && lc.contains("limit") {
        return ClaudeCliError::RateLimited {
            retry_after_seconds: 60,
        };
    }

    let snippet: String = msg.chars().take(256).collect();
    ClaudeCliError::Transport(if snippet.is_empty() {
        "claude reported error with no message".into()
    } else {
        snippet
    })
}
