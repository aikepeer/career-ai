//! The `Submitter` trait. All apply channels implement this.
//!
//! Dry-run is the default. Real submission gated by `auto_submit = true`
//! AND the per-source `submit_enabled` flag.

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("rate limit exceeded")]
    RateLimited,
    #[error("submission blocked by dry-run mode")]
    DryRun,
    #[error("source disabled in config")]
    Disabled,
    #[error("submission failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Submitter: Send + Sync {
    fn name(&self) -> &'static str;

    /// Apply to a single prepared application. Dry-run honored inside the
    /// concrete implementation, not at the call site.
    async fn submit(&self, application_id: &str) -> Result<(), SubmitError>;
}
