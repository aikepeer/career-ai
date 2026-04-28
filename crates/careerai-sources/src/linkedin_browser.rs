//! Native LinkedIn browser-driven discovery adapter.
//!
//! Drives a stealth Chromium session against the public
//! `linkedin.com/jobs/search/` page, paginates over results, and
//! emits one `RawListing` per job card. Reuses the M5 submitter's
//! `BrowserSession` (stealth-v2.js + cookie API) and `Credential`
//! store (`li_at` cookie loaded from the OS keychain) — see
//! `careerai_submit::browser_session` and `careerai_submit::credentials`.
//! No submission code paths are invoked; the dep is purely for
//! infrastructure reuse.
//!
//! Feature-gated on `browser`. The default build never pulls in
//! chromiumoxide or careerai-submit.
//!
//! ## Selectors
//!
//! All DOM selectors live in [`crate::linkedin_browser_selectors`]
//! (feature-flag-FREE) so the offline regression test in
//! `tests/linkedin_browser_fixture_it.rs` exercises the same
//! expressions the runtime uses.
//!
//! ## ToS posture
//!
//! LinkedIn User Agreement §8.2 forbids automated access; the user
//! opts in by flipping `sources.linkedin_browser.enabled = true`.
//! Defaults are conservative (3 pages max, 2 calls/min, 1.5–3.5s
//! inter-page jitter).

#![cfg(feature = "browser")]

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use careerai_core::config::LinkedinBrowserSourceConfig;
use careerai_submit::browser_session::{BrowserSession, BrowserSessionConfig};
use careerai_submit::credentials;
use careerai_submit::Credential;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use rand::Rng;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::base::{RawListing, Source, SourceError};
use crate::linkedin_browser_parser::{build_search_url, parse_search_html, KNOWN_EXPERIENCE_LEVELS};

const COOKIE_DOMAIN: &str = ".linkedin.com";
const COOKIE_NAME: &str = "li_at";

/// `Source::name()` value. Distinct from the listing-source value
/// (`"linkedin"`, set inside the parser) so the scheduler can
/// schedule discovery independently of submission and CLI filters
/// like `careerai discover --source linkedin-browser` don't collide
/// with M5 submitter naming.
const SOURCE_NAME: &str = "linkedin-browser";

/// Inter-page navigation jitter in milliseconds. Randomized
/// per-page-load so two consecutive ticks never have identical
/// timing fingerprints.
const PAGE_JITTER_MS_MIN: u64 = 1_500;
const PAGE_JITTER_MS_MAX: u64 = 3_500;

/// Wall-clock cap for a full `discover()` call. A misbehaving
/// LinkedIn (CAPTCHA loop, infinite redirect) would otherwise hold
/// the cron tick.
const DISCOVER_TIMEOUT_SECONDS: u64 = 180;

type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

/// Cached `Arc<RateLimiter>` so reconstructions across cron ticks
/// share the token bucket. Single instance: there's only one
/// `linkedin-browser` source per process. Mirrors `mcp_jobs::SOURCE_STATE`'s
/// rationale (build_sources runs every tick).
static RATE_LIMITER: std::sync::OnceLock<std::sync::Mutex<Option<Arc<ReadRateLimiter>>>> =
    std::sync::OnceLock::new();

fn get_rate_limiter(rate_per_minute: u32) -> Option<Arc<ReadRateLimiter>> {
    let cell = RATE_LIMITER.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_some() {
        return guard.clone();
    }
    let rl = NonZeroU32::new(rate_per_minute)
        .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
    guard.clone_from(&rl);
    rl
}

#[derive(Debug)]
pub struct LinkedinBrowserSource {
    cfg: LinkedinBrowserSourceConfig,
    rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl LinkedinBrowserSource {
    #[must_use]
    pub fn new(cfg: LinkedinBrowserSourceConfig) -> Self {
        // Drop unknown experience levels at construction so the URL
        // builder never emits them. Logged once so an operator typo
        // is visible in startup output.
        let mut sanitized = cfg;
        sanitized.filters.experience_level.retain(|lvl| {
            let lvl_lc = lvl.to_ascii_lowercase();
            let known = KNOWN_EXPERIENCE_LEVELS
                .iter()
                .any(|(name, _)| *name == lvl_lc);
            if !known {
                warn!(
                    target: "linkedin-browser",
                    level = %lvl,
                    "ignoring unknown experience_level (expected one of internship/entry/associate/mid/senior/director/executive)"
                );
            }
            known
        });
        let rate_limiter = get_rate_limiter(sanitized.rate_per_minute);
        Self {
            cfg: sanitized,
            rate_limiter,
        }
    }
}

#[async_trait]
impl Source for LinkedinBrowserSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let outcome = tokio::time::timeout(
            Duration::from_secs(DISCOVER_TIMEOUT_SECONDS),
            run_discover(self),
        )
        .await
        .map_err(|_| {
            SourceError::Parse(format!(
                "linkedin-browser discover timed out after {DISCOVER_TIMEOUT_SECONDS}s"
            ))
        })??;
        Ok(outcome)
    }
}

async fn run_discover(src: &LinkedinBrowserSource) -> Result<Vec<RawListing>, SourceError> {
    // Load credentials BEFORE spawning Chromium so a missing cookie
    // surfaces as a clean SourceError without paying the browser-launch
    // cost.
    let li_at = credentials::load(&Credential::for_source("linkedin", COOKIE_NAME))
        .map_err(|e| SourceError::Parse(format!("linkedin-browser cookie load failed: {e}")))?;

    let session_cfg = BrowserSessionConfig {
        headless: src.cfg.headless,
        request_timeout_seconds: src.cfg.action_timeout_seconds,
        ..BrowserSessionConfig::default()
    };

    let session = BrowserSession::launch(&session_cfg).await.map_err(|e| {
        SourceError::Parse(format!(
            "linkedin-browser launch failed (is Chromium installed?): {e}"
        ))
    })?;

    // Always close the browser, even on error, so we don't leak
    // Chromium processes across cron ticks.
    let inner = scrape_pages(src, &session, &li_at).await;
    if let Err(e) = session.close().await {
        warn!(
            target: "linkedin-browser",
            error = %e,
            "browser close failed (continuing with scraped results)"
        );
    }
    inner
}

async fn scrape_pages(
    src: &LinkedinBrowserSource,
    session: &BrowserSession,
    li_at: &str,
) -> Result<Vec<RawListing>, SourceError> {
    // Cookie must be set before navigating to a gated URL — otherwise
    // LinkedIn redirects to /login. Same posture as the M5 submitter.
    session
        .set_cookie(COOKIE_NAME, li_at, COOKIE_DOMAIN)
        .await
        .map_err(|e| SourceError::Parse(format!("linkedin-browser cookie install failed: {e}")))?;

    let pages = src.cfg.max_pages.max(1);
    let mut all: Vec<RawListing> = Vec::new();

    for page_idx in 0..pages {
        if let Some(rl) = &src.rate_limiter {
            // Read-side permit. `until_ready` blocks for at most
            // `1/rate_per_minute` so a misconfigured cap can't stall
            // the cron tick beyond the per-minute quota.
            rl.until_ready().await;
        }

        let url = build_search_url(&src.cfg, page_idx);
        debug!(
            target: "linkedin-browser",
            page = page_idx + 1,
            url = %url,
            "navigating to page"
        );
        session.navigate(&url).await.map_err(|e| {
            SourceError::Parse(format!(
                "linkedin-browser nav page={} failed: {e}",
                page_idx + 1
            ))
        })?;

        let html = capture_outer_html(session).await?;
        let parsed = parse_search_html(&html);
        info!(
            target: "linkedin-browser",
            page = page_idx + 1,
            cards = parsed.len(),
            "scraped page"
        );
        all.extend(parsed);

        // Inter-page delay (randomized) to avoid an obvious
        // mechanical-cadence fingerprint. Skipped on the last page.
        if page_idx + 1 < pages {
            let jitter_ms = jitter_ms();
            sleep(Duration::from_millis(jitter_ms)).await;
        }
    }

    Ok(all)
}

fn jitter_ms() -> u64 {
    let mut rng = rand::rng();
    rng.random_range(PAGE_JITTER_MS_MIN..=PAGE_JITTER_MS_MAX)
}

/// Pull the post-render HTML of the current page. chromiumoxide 0.7
/// exposes `Page::content()` which serializes the live DOM (post-JS,
/// post-XHR) and returns it as a string. The offline parser
/// (`linkedin_browser_parser::parse_search_html`) consumes that
/// string verbatim, so the same code path runs in tests against
/// captured fixture HTML.
async fn capture_outer_html(session: &BrowserSession) -> Result<String, SourceError> {
    session
        .page()
        .content()
        .await
        .map_err(|e| SourceError::Parse(format!("linkedin-browser page content fetch failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use careerai_core::config::LinkedinBrowserFilters;

    #[test]
    fn unknown_experience_level_dropped_at_construction() {
        let cfg = LinkedinBrowserSourceConfig {
            filters: LinkedinBrowserFilters {
                experience_level: vec!["wizard".to_string(), "mid".to_string()],
                ..LinkedinBrowserFilters::default()
            },
            ..LinkedinBrowserSourceConfig::default()
        };
        let src = LinkedinBrowserSource::new(cfg);
        assert_eq!(src.cfg.filters.experience_level, vec!["mid".to_string()]);
    }

    #[test]
    fn source_name_is_linkedin_browser() {
        let src = LinkedinBrowserSource::new(LinkedinBrowserSourceConfig::default());
        assert_eq!(src.name(), "linkedin-browser");
    }
}
