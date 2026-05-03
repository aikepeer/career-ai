use std::num::NonZeroU32;
use std::sync::Arc;

use async_trait::async_trait;
use careerai_core::config::LinkedinBrowserSourceConfig;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use tracing::warn;

use crate::base::{RawListing, Source, SourceError};
use crate::linkedin_browser_parser::KNOWN_EXPERIENCE_LEVELS;

use super::scrape::run_discover;

/// `Source::name()` value.
const SOURCE_NAME: &str = "linkedin-browser";

/// Wall-clock cap for a full `discover()` call.
const DISCOVER_TIMEOUT_SECONDS: u64 = 180;

/// Hard ceiling on `max_pages`.
pub const MAX_PAGES_CEILING: u32 = 10;

type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

static RATE_LIMITER: std::sync::OnceLock<std::sync::Mutex<Option<Arc<ReadRateLimiter>>>> =
    std::sync::OnceLock::new();

fn get_rate_limiter(rate_per_minute: u32) -> Option<Arc<ReadRateLimiter>> {
    let cell = RATE_LIMITER.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    pub(crate) cfg: LinkedinBrowserSourceConfig,
    pub(crate) rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl LinkedinBrowserSource {
    #[must_use]
    pub fn new(cfg: LinkedinBrowserSourceConfig) -> Self {
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
                    "ignoring unknown experience_level"
                );
            }
            known
        });
        if sanitized.max_pages > MAX_PAGES_CEILING {
            warn!(
                target: "linkedin-browser",
                requested = sanitized.max_pages,
                ceiling = MAX_PAGES_CEILING,
                "max_pages exceeds safety ceiling; clamping"
            );
            sanitized.max_pages = MAX_PAGES_CEILING;
        }
        let rate_limiter = get_rate_limiter(sanitized.rate_per_minute);
        if rate_limiter.is_none() {
            warn!(
                target: "linkedin-browser",
                "rate_per_minute=0 — read-side rate limiter disabled"
            );
        }
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
            std::time::Duration::from_secs(DISCOVER_TIMEOUT_SECONDS),
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
