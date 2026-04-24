//! Dry-run wrapper that short-circuits `Submitter::submit` and emits a
//! structured `would_submit` tracing event instead of touching the
//! network. Wraps any `Submitter`; the inner impl only has to get
//! `prepare()` right.

use async_trait::async_trait;
use tracing::info;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::Result;

/// Wraps any `Submitter` and forces dry-run behavior on every call.
#[derive(Debug)]
pub struct DryRunSubmitter<S: Submitter> {
    inner: S,
}

impl<S: Submitter> DryRunSubmitter<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// Expose the wrapped submitter for introspection (e.g. tests).
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

#[async_trait]
impl<S: Submitter> Submitter for DryRunSubmitter<S> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        self.inner.prepare(ctx)
    }

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        let would = self.inner.prepare(ctx)?;
        info!(
            target: "submit",
            event = "would_submit",
            source = would.source,
            url = %would.url,
            method = would.method,
            body_preview = %would.body_preview,
            artifacts = ?would.artifact_kinds,
            "dry-run: would submit (no network write)"
        );
        Ok(format!("dry-run:{}", would.source))
    }
}
