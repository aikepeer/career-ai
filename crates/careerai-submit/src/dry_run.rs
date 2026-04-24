//! Dry-run wrapper that short-circuits `Submitter::submit` and emits a
//! structured `would_submit` tracing event instead of touching the
//! network. Wraps any `Submitter` via `Box<dyn Submitter>` — the inner
//! impl only has to get `prepare()` right.

use async_trait::async_trait;
use tracing::info;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::Result;

/// Wraps any `Submitter` and forces dry-run behavior on every call.
///
/// Holds a `Box<dyn Submitter>` directly (no extra newtype) so callers
/// can drop in any concrete or already-erased submitter without an
/// adapter shim.
pub struct DryRunSubmitter {
    inner: Box<dyn Submitter>,
}

impl DryRunSubmitter {
    #[must_use]
    pub fn new(inner: Box<dyn Submitter>) -> Self {
        Self { inner }
    }

    /// Borrow the wrapped submitter (e.g. for tests).
    #[must_use]
    pub fn inner(&self) -> &dyn Submitter {
        self.inner.as_ref()
    }
}

impl std::fmt::Debug for DryRunSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DryRunSubmitter")
            .field("name", &self.inner.name())
            .finish()
    }
}

#[async_trait]
impl Submitter for DryRunSubmitter {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        self.inner.prepare(ctx)
    }

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        let would = self.inner.prepare(ctx)?;
        log_would_submit(&would, ctx);
        Ok(format!("dry-run:{}:{}", would.source, ctx.application.id))
    }
}

/// Emit the structured `would_submit` tracing event. PII fields
/// (`name`, `email`, `phone`, full cover-letter text) are deliberately
/// NOT logged — `body_preview` is omitted from the log line. Logs ship
/// to aggregators; bodies stay in the DB payload row where access is
/// gated.
pub(crate) fn log_would_submit(would: &WouldSubmit, ctx: &SubmitContext<'_>) {
    info!(
        target: "submit",
        event = "would_submit",
        source = would.source,
        url = %would.url,
        method = would.method,
        application_id = %ctx.application.id,
        listing_id = %ctx.listing.id,
        body_bytes = would.body_preview.len(),
        artifacts = ?would.artifact_kinds,
        "dry-run: would submit (no network write, PII omitted from log)"
    );
}
