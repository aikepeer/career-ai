use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

use crate::ats_http::sanitize_external_id;
use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::browser_session::{BrowserSession, BrowserSessionConfig};
use crate::credentials::{self, Credential};
use crate::error::{Result, SubmitError};
use crate::rate_limiter::{RateLimiter, RatePermit};

use super::config::LinkedinConfig;

// Re-export the always-on selector const so this module's call sites
// don't reach into `crate::linkedin_selectors`.
use crate::linkedin_selectors::EASY_APPLY_SELECTOR;

const LINKEDIN_DOMAIN: &str = ".linkedin.com";
const LINKEDIN_LI_AT: &str = "li_at";
const LINKEDIN_BASE: &str = "https://www.linkedin.com";

/// LinkedIn Easy Apply submitter.
pub struct LinkedinSubmitter {
    cfg: LinkedinConfig,
    rate_limiter: Arc<RateLimiter>,
}

impl std::fmt::Debug for LinkedinSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkedinSubmitter")
            .field("headless", &self.cfg.headless)
            .field("max_per_day", &self.cfg.rate_policy.max_per_day)
            .finish_non_exhaustive()
    }
}

impl LinkedinSubmitter {
    #[must_use]
    pub fn new(cfg: LinkedinConfig, rate_limiter: Arc<RateLimiter>) -> Self {
        Self { cfg, rate_limiter }
    }

    #[allow(clippy::unused_self)]
    fn would_submit_for(&self, ctx: &SubmitContext<'_>) -> WouldSubmit {
        let safe_id = sanitize_external_id(&ctx.listing.external_id);
        let url = format!("{LINKEDIN_BASE}/jobs/view/{safe_id}/");
        let preview = serde_json::to_string(&LinkedinPayloadPreview {
            listing_id: &ctx.listing.id,
            listing_title: &ctx.listing.title,
            listing_company: &ctx.listing.company,
            artifact_kinds: ctx.artifacts.iter().map(|a| a.kind.as_str()).collect(),
        })
        .unwrap_or_default();
        WouldSubmit {
            source: "linkedin",
            url,
            method: "BROWSER",
            body_preview: preview,
            artifact_kinds: ctx.artifacts.iter().map(|a| a.kind.clone()).collect(),
        }
    }

    #[allow(clippy::unused_self)]
    fn load_li_at(&self) -> Result<String> {
        credentials::load(&Credential::for_source("linkedin", LINKEDIN_LI_AT))
    }

    fn screenshot_path(&self, ctx: &SubmitContext<'_>, stage: &str) -> PathBuf {
        self.cfg
            .screenshots_dir
            .join(format!("{}-{}.png", ctx.application.id, stage))
    }
}

#[derive(Serialize)]
struct LinkedinPayloadPreview<'a> {
    listing_id: &'a str,
    listing_title: &'a str,
    listing_company: &'a str,
    artifact_kinds: Vec<&'a str>,
}

#[async_trait]
impl Submitter for LinkedinSubmitter {
    fn name(&self) -> &'static str {
        "linkedin"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        Ok(self.would_submit_for(ctx))
    }

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        let permit = self
            .rate_limiter
            .acquire(self.name(), &self.cfg.rate_policy)
            .await
            .map_err(|e| SubmitError::SourceDisabled(format!("rate-limited: {e}")))?;

        let li_at = self.load_li_at()?;

        let mut session_cfg = BrowserSessionConfig {
            headless: self.cfg.headless,
            request_timeout_seconds: self.cfg.action_timeout_seconds,
            ..BrowserSessionConfig::default()
        };
        if let Some(ua) = &self.cfg.user_agent {
            session_cfg.user_agent.clone_from(ua);
        }

        let session = BrowserSession::launch(&session_cfg)
            .await
            .map_err(|e| match e {
                SubmitError::Io(io) => SubmitError::SourceDisabled(format!(
                    "linkedin browser launch failed (is Chromium installed?): {io}"
                )),
                other => other,
            })?;

        let mut permit_slot = Some(permit);
        let outcome = self
            .run_session(&session, ctx, &li_at, &mut permit_slot)
            .await;
        if let Err(e) = session.close().await {
            tracing::warn!(target: "submit", error = %e, "linkedin browser close failed");
        }
        outcome
    }
}

impl LinkedinSubmitter {
    async fn run_session(
        &self,
        session: &BrowserSession,
        ctx: &SubmitContext<'_>,
        li_at: &str,
        permit_slot: &mut Option<RatePermit<'_>>,
    ) -> Result<String> {
        session
            .set_cookie(LINKEDIN_LI_AT, li_at, LINKEDIN_DOMAIN)
            .await?;

        let safe_id = sanitize_external_id(&ctx.listing.external_id);
        let url = format!("{LINKEDIN_BASE}/jobs/view/{safe_id}/");
        session.navigate(&url).await?;

        let landing_path = self.screenshot_path(ctx, "landing");
        if let Err(e) = ensure_parent(&landing_path).await {
            tracing::warn!(
                target: "submit",
                error = %e,
                path = %landing_path.display(),
                "linkedin screenshot dir create failed"
            );
        }
        if let Err(e) = session.screenshot(&landing_path).await {
            tracing::warn!(
                target: "submit",
                error = %e,
                path = %landing_path.display(),
                "linkedin landing screenshot failed"
            );
        }

        click_easy_apply(session, self.cfg.action_timeout_seconds).await?;

        if let Some(permit) = permit_slot.take() {
            permit.commit();
        }

        let final_path = self.screenshot_path(ctx, "pre-submit");
        if let Err(e) = session.screenshot(&final_path).await {
            tracing::warn!(
                target: "submit",
                error = %e,
                path = %final_path.display(),
                "linkedin pre-submit screenshot failed"
            );
        }

        if !self.cfg.allow_submit_click {
            return Err(SubmitError::SourceDisabled(format!(
                "linkedin submit reached pre-submit state (screenshot: {}); \
                 final Submit click gated by submit.linkedin.allow_submit_click=false — \
                 see CLAUDE.md ToS notes (M5b lifts after audit)",
                final_path.display()
            )));
        }

        Err(SubmitError::SourceDisabled(format!(
            "linkedin submit reached pre-submit state (screenshot: {}); \
             final Submit click is intentionally not implemented in M5a; \
             allow_submit_click=true is reserved for M5b",
            final_path.display()
        )))
    }
}

async fn ensure_parent(p: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = p.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

async fn click_easy_apply(session: &BrowserSession, timeout_seconds: u64) -> Result<()> {
    let timeout = Duration::from_secs(timeout_seconds);

    let element_fut = session.page().find_element(EASY_APPLY_SELECTOR);
    let element = match tokio::time::timeout(timeout, element_fut).await {
        Ok(Ok(el)) => el,
        Ok(Err(e)) => {
            return Err(SubmitError::SourceDisabled(format!(
                "linkedin Easy Apply button not found ({EASY_APPLY_SELECTOR}): {e}"
            )));
        }
        Err(_) => {
            return Err(SubmitError::SourceDisabled(format!(
                "linkedin Easy Apply button not found within {timeout_seconds}s; \
                 listing may be closed, region-gated, or a CAPTCHA appeared"
            )));
        }
    };
    let click_fut = element.click();
    match tokio::time::timeout(timeout, click_fut).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(SubmitError::SourceDisabled(format!(
            "linkedin Easy Apply click failed: {e}"
        ))),
        Err(_) => Err(SubmitError::SourceDisabled(format!(
            "linkedin Easy Apply click stalled ({timeout_seconds}s)"
        ))),
    }
}
