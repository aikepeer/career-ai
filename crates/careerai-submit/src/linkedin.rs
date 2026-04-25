//! LinkedIn Easy Apply browser submitter.
//!
//! Drives a chromiumoxide-launched Chromium through the multi-step
//! Easy Apply modal. Auto-submit is gated by:
//!   1. `SubmitConfig.auto_submit = true` (top-level kill switch)
//!   2. `submit.per_source.linkedin.enabled = true` (per-source switch)
//!   3. Rate limiter permit granted (max_per_day + min-interval +
//!      quiet hours)
//!
//! In dry-run mode the dispatcher (`submit_application`) calls
//! `prepare()` only — no browser, no I/O. The live `submit()` path
//! deliberately stops one click short of the final "Submit application"
//! button: it acquires a rate-limit permit, loads the `li_at` cookie,
//! launches stealth Chromium, navigates, opens the Easy Apply modal,
//! takes a pre-submit screenshot, then returns
//! `SubmitError::SourceDisabled` pointing at the screenshot. M5b will
//! enable the final click in a separate auditable commit per the
//! CLAUDE.md ToS-mitigation policy.

#![cfg(feature = "browser")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::browser_session::{BrowserSession, BrowserSessionConfig};
use crate::credentials::{self, Credential};
use crate::error::{Result, SubmitError};
use crate::rate_limiter::{RateLimiter, RatePolicy};

const LINKEDIN_DOMAIN: &str = ".linkedin.com";
const LINKEDIN_LI_AT: &str = "li_at";
const LINKEDIN_BASE: &str = "https://www.linkedin.com";

/// Configuration for `LinkedinSubmitter`. Mirrors the YAML shape under
/// `config.linkedin.<...>` — Wave 3 wires this from `CoreConfig`.
#[derive(Debug, Clone)]
pub struct LinkedinConfig {
    /// Where screenshots land. The submitter writes
    /// `<screenshots_dir>/<application_id>-<stage>.png` and embeds the
    /// path into the returned error so an operator can audit the run.
    pub screenshots_dir: PathBuf,
    /// Optional override for the Chromium runtime user-agent. Defaults
    /// to the modern Linux Chrome UA from `BrowserSessionConfig`.
    pub user_agent: Option<String>,
    /// Headless or headed. Default `true`.
    pub headless: bool,
    /// Rate-limit policy. Sane default: 10/day, 120s between, 60s
    /// jitter, 19:00..01:00 UTC quiet hours (favors IST-night
    /// submission).
    pub rate_policy: RatePolicy,
    /// Selector / action timeout in seconds. Default 20s.
    pub action_timeout_seconds: u64,
}

impl Default for LinkedinConfig {
    fn default() -> Self {
        Self {
            screenshots_dir: PathBuf::from("artifacts/screenshots/linkedin"),
            user_agent: None,
            headless: true,
            rate_policy: RatePolicy {
                max_per_day: 10,
                min_seconds_between: 120,
                jitter_seconds: 60,
                // 19:00 UTC = 00:30 IST. Quiet hours run from 19:00 UTC
                // through 01:00 UTC, covering late IST night when no
                // human operator would be reviewing submissions.
                quiet_hours_utc: Some((19, 1)),
            },
            action_timeout_seconds: 20,
        }
    }
}

/// LinkedIn Easy Apply submitter. Holds a shared `RateLimiter` so
/// concurrent invocations (e.g. `apply --all`) coordinate one bucket.
pub struct LinkedinSubmitter {
    cfg: LinkedinConfig,
    rate_limiter: Arc<RateLimiter>,
}

impl std::fmt::Debug for LinkedinSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `screenshots_dir` (could leak a user path)
        // and the `user_agent` override (low value, possibly long).
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

    /// Build a `WouldSubmit` envelope. Pure (no I/O) so the dry-run
    /// path can call this without ever spawning a browser. Method (vs
    /// free fn) for cohesion + so M5b's screening-question handler can
    /// add config-driven preview shaping here.
    #[allow(clippy::unused_self)]
    fn would_submit_for(&self, ctx: &SubmitContext<'_>) -> WouldSubmit {
        let url = format!("{LINKEDIN_BASE}/jobs/view/{}/", ctx.listing.external_id);
        // Body preview is structural only — the submit-layer logging
        // policy says we never put email/phone/cover-letter text into
        // log lines; payload stays in the application_payloads DB row.
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

    /// Load LinkedIn `li_at` session cookie from the credentials store.
    /// Returns `SubmitError::SourceDisabled` with an actionable message
    /// when missing — same shape used elsewhere in the submit layer.
    /// Method (vs free fn) so a future per-instance credential override
    /// (e.g. multiple LinkedIn accounts) plugs in here.
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

    /// Live path. NOT called when the dispatcher routes through
    /// `DryRunSubmitter` — that wrapper invokes `prepare()` only.
    ///
    /// 1. Acquire a rate-limit permit (denied → SourceDisabled).
    /// 2. Load the `li_at` cookie from the keychain.
    /// 3. Launch stealth Chromium.
    /// 4. Set the cookie on `.linkedin.com`.
    /// 5. Navigate to the listing.
    /// 6. Click the Easy Apply CTA.
    /// 7. Take a pre-submit screenshot and bail with SourceDisabled
    ///    pointing at the screenshot — M5b will enable the final
    ///    Submit click in a separate commit.
    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        // Rate limit gate first — denied permits don't cost a browser
        // spawn.
        self.rate_limiter
            .acquire(self.name(), &self.cfg.rate_policy)
            .await
            .map_err(|e| SubmitError::SourceDisabled(format!("rate-limited: {e}")))?;

        let li_at = self.load_li_at()?;

        // Build session config from our linkedin config.
        let mut session_cfg = BrowserSessionConfig {
            headless: self.cfg.headless,
            launch_timeout_seconds: self.cfg.action_timeout_seconds,
            ..BrowserSessionConfig::default()
        };
        if let Some(ua) = &self.cfg.user_agent {
            session_cfg.user_agent.clone_from(ua);
        }

        let session = BrowserSession::launch(&session_cfg).await.map_err(|e| {
            // Re-wrap so the top-level error carries an actionable hint
            // about the most common failure mode (Chromium not on PATH).
            match e {
                SubmitError::Io(io) => SubmitError::SourceDisabled(format!(
                    "linkedin browser launch failed (is Chromium installed?): {io}"
                )),
                other => other,
            }
        })?;

        // Cookie must be set BEFORE navigating to a gated page;
        // otherwise LinkedIn redirects to /login and we lose state.
        session
            .set_cookie(LINKEDIN_LI_AT, &li_at, LINKEDIN_DOMAIN, true)
            .await?;

        let url = format!("{LINKEDIN_BASE}/jobs/view/{}/", ctx.listing.external_id);
        session.navigate(&url).await?;

        // Audit screenshot before any interaction. Captures the listing
        // as the operator would see it; useful when the Easy Apply
        // button isn't present (closed listing, region gate).
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

        // Easy Apply state machine. M5a only handles the happy path
        // (no screening questions). Screening-question handling lands
        // in M5b alongside the actual submit click.
        click_easy_apply(&session, self.cfg.action_timeout_seconds).await?;

        // Pre-submit screenshot — we never click the final Submit
        // button in M5a even on the live path. The infrastructure is
        // in place; clicking is a separate auditable commit.
        let final_path = self.screenshot_path(ctx, "pre-submit");
        if let Err(e) = session.screenshot(&final_path).await {
            tracing::warn!(
                target: "submit",
                error = %e,
                path = %final_path.display(),
                "linkedin pre-submit screenshot failed"
            );
        }

        Err(SubmitError::SourceDisabled(format!(
            "linkedin submit reached pre-submit state (screenshot: {}); \
             clicking the final Submit button is gated to M5b — see CLAUDE.md \
             ToS notes",
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
    // Match on attribute presence rather than exact class — LinkedIn
    // ships a few CTA variants depending on which A/B bucket the
    // account is in. A class rename should not break us silently.
    const SELECTOR: &str = "button.jobs-apply-button, button[aria-label*='Easy Apply']";
    // Reserved for future explicit poll-loops; kept for parity with
    // the spec and so a follow-up wave can reuse the deadline without
    // changing this signature.
    let _deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_seconds);

    let element = session.page().find_element(SELECTOR).await.map_err(|e| {
        SubmitError::SourceDisabled(format!(
            "linkedin Easy Apply button not found ({SELECTOR}): {e}"
        ))
    })?;
    element.click().await.map_err(|e| {
        SubmitError::SourceDisabled(format!("linkedin Easy Apply click failed: {e}"))
    })?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_db::models::{Application, Artifact, Listing};
    use careerai_profile::schema::Profile;
    use chrono::Utc;

    fn fixture_listing() -> Listing {
        let now = Utc::now();
        Listing {
            id: "l-1".into(),
            source: "linkedin".into(),
            external_id: "4123456789".into(),
            title: "Senior ML Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote, India".into()),
            url: "https://www.linkedin.com/jobs/view/4123456789".into(),
            description: "...".into(),
            raw_json: None,
            state: "rendered".into(),
            score: Some(0.9),
            created_at: now,
            updated_at: now,
        }
    }

    fn fixture_application(listing_id: &str) -> Application {
        let now = Utc::now();
        Application {
            id: "app-1".into(),
            listing_id: listing_id.into(),
            state: "rendered".into(),
            profile_hash: "sha256:x".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "claude-3-5-sonnet".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn fixture_profile() -> Profile {
        let mut p = Profile::default();
        p.personal.name = "Ada Applicant".into();
        p.personal.email = "ada@example.com".into();
        p
    }

    #[test]
    fn prepare_builds_listing_url_from_external_id() {
        let listing = fixture_listing();
        let application = fixture_application(&listing.id);
        let profile = fixture_profile();
        let artifacts: Vec<Artifact> = Vec::new();
        let ctx = SubmitContext {
            application: &application,
            listing: &listing,
            profile: &profile,
            artifacts: &artifacts,
            cover_letter_text: "test",
        };
        let sub = LinkedinSubmitter::new(LinkedinConfig::default(), Arc::new(RateLimiter::new()));
        let would = sub.prepare(&ctx).unwrap();
        assert_eq!(would.source, "linkedin");
        assert_eq!(would.method, "BROWSER");
        assert!(
            would.url.contains("/jobs/view/4123456789"),
            "url={}",
            would.url
        );
        // Never include PII in body_preview.
        assert!(
            !would.body_preview.contains("ada@example.com"),
            "leaked email: {}",
            would.body_preview
        );
        assert!(
            !would.body_preview.contains("Ada Applicant"),
            "leaked name: {}",
            would.body_preview
        );
        // DOES include structural fields.
        assert!(
            would.body_preview.contains("Senior ML Engineer"),
            "missing title: {}",
            would.body_preview
        );
        assert!(
            would.body_preview.contains("Acme"),
            "missing company: {}",
            would.body_preview
        );
    }

    #[test]
    fn debug_omits_full_config() {
        let sub = LinkedinSubmitter::new(LinkedinConfig::default(), Arc::new(RateLimiter::new()));
        let s = format!("{sub:?}");
        assert!(s.contains("LinkedinSubmitter"), "got: {s}");
        assert!(s.contains("max_per_day"), "got: {s}");
        // No screenshots_dir leaked (which could be a user path).
        assert!(!s.contains("screenshots_dir"), "leaked path: {s}");
    }
}
