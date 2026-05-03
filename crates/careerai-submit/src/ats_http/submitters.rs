use async_trait::async_trait;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::{Result, SubmitError};

use super::support::{
    artifact_kinds_owned, build_body_preview, slugify, CandidatePayload,
};

// NOTE: `sanitize_external_id` is re-exported from the root via `pub use`.

const GREENHOUSE_DEFAULT_BASE: &str = "https://boards-api.greenhouse.io";
const LEVER_DEFAULT_BASE: &str = "https://api.lever.co";
const ASHBY_DEFAULT_BASE: &str = "https://api.ashbyhq.com";

// --- Greenhouse ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GreenhouseSubmitter {
    base_url: String,
}

impl Default for GreenhouseSubmitter {
    fn default() -> Self {
        Self {
            base_url: GREENHOUSE_DEFAULT_BASE.to_owned(),
        }
    }
}

impl GreenhouseSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn post_url(&self, ctx: &SubmitContext<'_>) -> String {
        let company_slug = slugify(&ctx.listing.company);
        format!(
            "{}/v1/boards/{}/jobs/{}",
            self.base_url.trim_end_matches('/'),
            company_slug,
            super::support::sanitize_external_id(&ctx.listing.external_id)
        )
    }
}

#[async_trait]
impl Submitter for GreenhouseSubmitter {
    fn name(&self) -> &'static str {
        "greenhouse"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        let body_preview = build_body_preview(&payload)?;
        Ok(WouldSubmit {
            source: "greenhouse",
            url: self.post_url(ctx),
            method: "POST",
            body_preview,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::SourceDisabled(
            "greenhouse live submit requires Harvest API key — use dry-run or the browser flow in M5".to_owned(),
        ))
    }
}

// --- Lever -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LeverSubmitter {
    base_url: String,
}

impl Default for LeverSubmitter {
    fn default() -> Self {
        Self {
            base_url: LEVER_DEFAULT_BASE.to_owned(),
        }
    }
}

impl LeverSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn post_url(&self, ctx: &SubmitContext<'_>) -> String {
        let company_slug = slugify(&ctx.listing.company);
        format!(
            "{}/v0/postings/{}/{}",
            self.base_url.trim_end_matches('/'),
            company_slug,
            super::support::sanitize_external_id(&ctx.listing.external_id)
        )
    }
}

#[async_trait]
impl Submitter for LeverSubmitter {
    fn name(&self) -> &'static str {
        "lever"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        let body_preview = build_body_preview(&payload)?;
        Ok(WouldSubmit {
            source: "lever",
            url: self.post_url(ctx),
            method: "POST",
            body_preview,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::SourceDisabled(
            "lever live submit requires Lever API key — use dry-run or the browser flow in M5"
                .to_owned(),
        ))
    }
}

// --- Ashby -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AshbySubmitter {
    base_url: String,
}

impl Default for AshbySubmitter {
    fn default() -> Self {
        Self {
            base_url: ASHBY_DEFAULT_BASE.to_owned(),
        }
    }
}

impl AshbySubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn post_url(&self) -> String {
        format!(
            "{}/applicationForm.submit",
            self.base_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl Submitter for AshbySubmitter {
    fn name(&self) -> &'static str {
        "ashby"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        let body_preview = build_body_preview(&payload)?;
        Ok(WouldSubmit {
            source: "ashby",
            url: self.post_url(),
            method: "POST",
            body_preview,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::SourceDisabled(
            "ashby live submit requires Ashby API key — use dry-run or the browser flow in M5"
                .to_owned(),
        ))
    }
}
