//! Domain error type for the MCP server.
//!
//! All tool handlers return `Result<_, McpServerError>` and convert into
//! `rmcp::ErrorData` at the boundary. The crate never panics on malformed
//! input; it surfaces a typed variant that the MCP client can render.

use rmcp::ErrorData;
use serde_json::json;
use thiserror::Error;

/// Single concrete error type returned from every tool handler.
#[derive(Debug, Error)]
pub enum McpServerError {
    /// The user passed an `apply` request with `dry_run: false` but did not
    /// also include the `confirm` token. This is a hard refusal — the
    /// server never auto-applies without explicit acknowledgement.
    #[error(
        "auto-submit refused: dry_run=false requires confirm=\"I_UNDERSTAND_TOS_RISK\" \
         (LinkedIn/Indeed auto-apply violates their ToS; this is a kill-switch)"
    )]
    AutoSubmitNotConfirmed,

    /// `careerai_profile_status` was called but `profile/profile.yaml` is
    /// missing or unreadable.
    #[error("profile not found at {path}: {source}")]
    ProfileMissing {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// `profile.yaml` exists but failed schema parsing or validation.
    #[error("profile invalid: {0}")]
    ProfileInvalid(String),

    /// A resource URI was malformed (e.g. `careerai://artifacts/abc` with
    /// a non-existent application id).
    #[error("resource not found: {0}")]
    ResourceNotFound(String),

    /// Wrapping any other failure (DB error, IO, pipeline error). Stringly
    /// because the underlying types are `anyhow::Error` from the pipeline
    /// crate; the MCP client receives the user-facing message only.
    #[error("pipeline error: {0}")]
    Pipeline(String),
}

impl From<anyhow::Error> for McpServerError {
    fn from(e: anyhow::Error) -> Self {
        Self::Pipeline(format!("{e:#}"))
    }
}

impl From<McpServerError> for ErrorData {
    fn from(e: McpServerError) -> Self {
        let message = e.to_string();
        match e {
            McpServerError::AutoSubmitNotConfirmed => ErrorData::invalid_params(
                message,
                Some(json!({
                    "hint": "pass confirm=\"I_UNDERSTAND_TOS_RISK\" alongside dry_run=false",
                })),
            ),
            McpServerError::ProfileMissing { path, .. } => {
                ErrorData::invalid_request(message, Some(json!({ "path": path })))
            }
            McpServerError::ProfileInvalid(_) => ErrorData::invalid_request(message, None),
            McpServerError::ResourceNotFound(uri) => {
                ErrorData::resource_not_found(message, Some(json!({ "uri": uri })))
            }
            McpServerError::Pipeline(_) => ErrorData::internal_error(message, None),
        }
    }
}
