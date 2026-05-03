//! Error types for the `claude` CLI subprocess driver.
//!
//! Mapped into [`LlmError`] at the trait boundary via `From`.

use crate::error::LlmError;

use super::response::ClaudeCliResult;

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
    let msg = parsed.result.clone().unwrap_or_default();
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
