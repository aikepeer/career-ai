//! Naukri.com browser-driven submitter. Mirrors the M5a LinkedIn pattern:
//! chromiumoxide session, governor rate-limiter, OS-keyring session
//! cookie, audit screenshot. Live-submits when `auto_submit=true` AND
//! `per_source.naukri.enabled=true`. No `interactive_only` gate (Naukri
//! risk profile is lower than LinkedIn per the design spec).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::browser_session::BrowserSession;
use crate::credentials::{self, Credential};
use crate::error::Result;
use crate::error::SubmitError;
use crate::naukri_selectors::{
    APPLIED_SUCCESS_SELECTOR, APPLY_BUTTON_SELECTOR, CONFIRM_APPLY_SELECTOR,
    LOGIN_REQUIRED_INDICATOR,
};
use crate::rate_limiter::{RateLimiter, RatePermit, RatePolicy};

/// Naukri keyring key name.
const NAUKRI_SESSION_KEY: &str = "session_cookie";
/// Cookie name used on naukri.com for session auth.
const NAUKRI_COOKIE_NAME: &str = "nauk_at";
/// Domain scope for the cookie injection.
const NAUKRI_DOMAIN: &str = ".naukri.com";

/// Naukri-specific runtime configuration, post-translation from
/// `careerai-core::config::NaukriSubmitConfig`.
#[derive(Debug, Clone)]
pub struct NaukriConfig {
    pub screenshots_dir: PathBuf,
    pub user_agent: Option<String>,
    pub headless: bool,
    pub rate_policy: RatePolicy,
    pub action_timeout_seconds: u64,
}

impl NaukriConfig {
    /// Build a `NaukriConfig` from the user's `SubmitConfig`.
    /// Reads the dedicated `submit.naukri` block; all fields have
    /// `#[serde(default)]` so a missing block yields the same shape
    /// as `NaukriConfig::default()`.
    #[must_use]
    pub fn from_core(submit_cfg: &careerai_core::config::SubmitConfig) -> Self {
        // .validated() clamps out-of-range / full-coverage quiet_hours_utc
        // to the default window with a warning. Mirrors the LinkedIn
        // pattern so a misconfigured `submit.naukri` block never reaches
        // the rate-limiter.
        let nk = submit_cfg.naukri.clone().validated();
        Self {
            screenshots_dir: nk.screenshots_dir,
            user_agent: nk.user_agent,
            headless: nk.headless,
            rate_policy: RatePolicy {
                max_per_day: nk.max_per_day,
                min_seconds_between: nk.min_seconds_between,
                jitter_seconds: nk.jitter_seconds,
                quiet_hours_utc: nk.quiet_hours_utc,
            },
            action_timeout_seconds: nk.action_timeout_seconds,
        }
    }

    /// Build the path for an audit screenshot. Used by `run_session` once
    /// the full chromiumoxide click flow is implemented.
    #[allow(dead_code)]
    fn screenshot_path(&self, ctx: &SubmitContext<'_>, stage: &str) -> PathBuf {
        self.screenshots_dir
            .join(format!("{}-{}.png", ctx.application.id, stage))
    }
}

impl Default for NaukriConfig {
    fn default() -> Self {
        Self {
            screenshots_dir: PathBuf::from("artifacts/naukri-audit"),
            user_agent: None,
            headless: true,
            rate_policy: RatePolicy {
                max_per_day: 10,
                min_seconds_between: 60,
                jitter_seconds: 15,
                quiet_hours_utc: Some((19, 1)),
            },
            action_timeout_seconds: 30,
        }
    }
}

#[derive(Debug)]
pub struct NaukriSubmitter {
    // Both fields are wired through `new()` and consumed by `run_session`,
    // which is currently dead code (the click flow ships in M5c). The
    // allow keeps the struct shape stable for the follow-up implementer
    // without spawning per-field allows.
    #[allow(dead_code)]
    cfg: NaukriConfig,
    #[allow(dead_code)]
    rate_limiter: Arc<RateLimiter>,
}

impl NaukriSubmitter {
    #[must_use]
    pub fn new(cfg: NaukriConfig, rate_limiter: Arc<RateLimiter>) -> Self {
        Self { cfg, rate_limiter }
    }

    /// Build a `WouldSubmit` envelope. Pure (no I/O) so the dry-run path can
    /// call this without spawning a browser.
    #[allow(clippy::unused_self)]
    fn would_submit_for(&self, ctx: &SubmitContext<'_>) -> WouldSubmit {
        WouldSubmit {
            source: "naukri",
            url: ctx.listing.url.clone(),
            method: "BROWSER",
            body_preview: format!(
                "{{\"listing_id\":{:?},\"title\":{:?},\"company\":{:?}}}",
                ctx.listing.id, ctx.listing.title, ctx.listing.company
            ),
            artifact_kinds: ctx.artifacts.iter().map(|a| a.kind.clone()).collect(),
        }
    }

    /// Load the Naukri session cookie from the OS keyring.
    /// Returns `SubmitError::SourceDisabled` with an actionable message when
    /// missing — same shape as LinkedIn's `load_li_at`.
    ///
    /// Currently unused — `submit()` short-circuits with `NotImplemented`
    /// before reaching this. Kept as the documented load path for the
    /// follow-up implementer.
    #[allow(clippy::unused_self, dead_code)]
    fn load_naukri_session(&self) -> Result<String> {
        credentials::load(&Credential::for_source("naukri", NAUKRI_SESSION_KEY))
    }

    /// Inner submit flow. Owns no resources; `submit()` ensures the
    /// `BrowserSession` is closed regardless of whether this returns Ok or Err.
    ///
    /// Currently unused — `submit()` returns `NotImplemented` before any
    /// browser launch. The function is kept (with `#[allow(dead_code)]`)
    /// as the documented click-flow reference for the follow-up
    /// implementer who lands the BrowserSession helper methods (click,
    /// wait_for, element_present).
    ///
    /// Expected click flow (documented here for the follow-up implementer):
    ///   1. `session.set_cookie(NAUKRI_COOKIE_NAME, cookie, NAUKRI_DOMAIN)`
    ///   2. `session.navigate(&ctx.listing.url)`
    ///   3. Screenshot landing page → `<screenshots_dir>/<id>-landing.png`
    ///   4. Check LOGIN_REQUIRED_INDICATOR — if present, return SourceDisabled
    ///      with "run `careerai cookies refresh naukri`" hint.
    ///   5. Click APPLY_BUTTON_SELECTOR.
    ///   6. Commit `permit_slot.take()` after first successful click.
    ///   7. Optionally click CONFIRM_APPLY_SELECTOR if modal appears.
    ///   8. Screenshot pre-submit state → `<screenshots_dir>/<id>-pre-submit.png`
    ///   9. Wait for APPLIED_SUCCESS_SELECTOR.
    ///  10. Return `ctx.listing.external_id` as the remote submission ID.
    #[allow(clippy::unused_async, dead_code)]
    async fn run_session(
        &self,
        session: &BrowserSession,
        ctx: &SubmitContext<'_>,
        cookie: &str,
        permit_slot: &mut Option<RatePermit<'_>>,
    ) -> Result<String> {
        let _ = (session, ctx, cookie, permit_slot);
        let _ = (
            APPLY_BUTTON_SELECTOR,
            CONFIRM_APPLY_SELECTOR,
            APPLIED_SUCCESS_SELECTOR,
            LOGIN_REQUIRED_INDICATOR,
            NAUKRI_COOKIE_NAME,
            NAUKRI_DOMAIN,
        );
        Err(SubmitError::NotImplemented(
            "naukri click flow not yet implemented — see naukri.rs::run_session for the \
             documented sequence; browser helper methods land in the follow-up commit"
                .into(),
        ))
    }
}

#[async_trait]
impl Submitter for NaukriSubmitter {
    fn name(&self) -> &'static str {
        "naukri"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        Ok(self.would_submit_for(ctx))
    }

    /// Live path. NOT called when the dispatcher routes through
    /// `DryRunSubmitter` — that wrapper invokes `prepare()` only.
    ///
    /// **Stub.** Until the chromiumoxide click flow lands (see
    /// `run_session` for the documented sequence and follow-up plan),
    /// this returns `NotImplemented` *before* any browser launch or
    /// rate-permit acquisition. Both of those have observable side
    /// effects — leaked Chromium processes and consumed daily quota —
    /// that must not happen for a path the engineer hasn't shipped.
    /// `NotImplemented` (vs `SourceDisabled`) keeps the audit log
    /// honest: the operator can tell "I haven't enabled this" from
    /// "career-ai hasn't built this yet".
    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::NotImplemented(
            "naukri submitter is a scaffold — the chromiumoxide click flow ships in M5c. \
             To unblock testing, set submit.per_source.naukri.enabled=false in your config."
                .into(),
        ))
    }
}
