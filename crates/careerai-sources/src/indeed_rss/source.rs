//! Indeed RSS source driver: HTTP client, rate limiting, discovery.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use careerai_core::config::IndeedRssSourceConfig;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use reqwest::Client;
use tracing::warn;

use crate::base::{RawListing, Source, SourceError};

use super::parser::parse_feed;

const DEFAULT_BASE_URL: &str = "https://rss.indeed.com";
pub(crate) const SOURCE_NAME: &str = "indeed_rss";

/// Hard ceiling on `discover()` HTTP wall time. Indeed's edge can stall
/// indefinitely on 5xx upstream incidents; without a timeout one bad
/// tick will pin a scheduler job until the daemon is killed.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// User-agent header sent with every request. RemoteOK hard-rejects
/// UA-less requests; Indeed's behavior is undocumented but its CDN is
/// known to throttle anonymous traffic. Pinning the same UA the rest
/// of the workspace uses keeps logs greppable and traffic identifiable.
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 (careerai/0.1)";

/// Indeed's `fromage` parameter accepts an integer "posted within N
/// days" window. Empirically the upstream accepts 1..=30; values above
/// 30 are silently treated as 30, and `0` returns nothing useful. We
/// clamp at the adapter boundary so a typo in `local.yaml` produces a
/// usable feed instead of an empty one.
const FROMAGE_MIN: u32 = 1;
const FROMAGE_MAX: u32 = 30;

/// Per-instance read-side rate limiter. Same shape as `mcp_jobs.rs` —
/// see that module's comment for why we don't reuse the submit-side
/// limiter here.
type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

/// Per-source-name interner for the read-side rate limiter, mirroring the
/// `mcp_jobs.rs` pattern (see lines 74–107 there for the rationale).
///
/// `careerai-pipeline::build_sources` reconstructs `IndeedRssSource` on
/// every cron tick. Without this cache, each tick rebuilds the limiter
/// with a full bucket and `rate_per_minute` is effectively unenforced in
/// daemon mode. Holding the `Arc<RateLimiter>` in a process-wide
/// `OnceLock<Mutex<HashMap>>` keyed by source name means the bucket state
/// survives reconstruction — the per-minute cap is honored across ticks.
///
/// Today there is exactly one Indeed RSS source (`SOURCE_NAME`), but the
/// keying matches `mcp_jobs.rs` so adding multi-instance support later is
/// a non-event.
type InternMap = std::sync::Mutex<std::collections::HashMap<String, Option<Arc<ReadRateLimiter>>>>;
static SOURCE_STATE: std::sync::OnceLock<InternMap> = std::sync::OnceLock::new();

pub(crate) fn intern_rate_limiter(
    name: &str,
    rate_per_minute: u32,
) -> Option<Arc<ReadRateLimiter>> {
    let map = SOURCE_STATE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // `lock()` only fails if a previous holder panicked. The cached state
    // is `Option<Arc<RateLimiter>>`; recovery is safe.
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = guard.get(name) {
        return entry.clone();
    }
    let rl = NonZeroU32::new(rate_per_minute)
        .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
    guard.insert(name.to_owned(), rl.clone());
    rl
}

#[derive(Debug)]
pub struct IndeedRssSource {
    cfg: IndeedRssSourceConfig,
    base_url: String,
    http: Client,
    /// Shared with all other `IndeedRssSource` instances constructed with
    /// the same source name (today: always `SOURCE_NAME`). The
    /// token-bucket state lives in the `Arc`, so a new construction on
    /// the next cron tick via `build_sources()` does not reset the
    /// bucket — the per-minute cap is honored across ticks.
    rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl IndeedRssSource {
    /// Build a source from its config block. The HTTP client carries a
    /// fixed `HTTP_TIMEOUT` and a stable `USER_AGENT`; if those static
    /// settings are unrepresentable, reqwest's TLS / runtime init is
    /// broken and nothing else in the binary will work either, so we
    /// panic visibly rather than degrade silently.
    #[must_use]
    pub fn new(cfg: IndeedRssSourceConfig) -> Self {
        let rate_limiter = intern_rate_limiter(SOURCE_NAME, cfg.rate_per_minute);
        #[allow(clippy::expect_used)]
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("build reqwest client with static UA + timeout");
        Self {
            cfg,
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
            rate_limiter,
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for IndeedRssSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        if let Some(rl) = &self.rate_limiter {
            // Read-side permit. `until_ready` returns when the bucket
            // refills, bounded by `1 / rate_per_minute`.
            rl.until_ready().await;
        }

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/rss");
        let mut req = self
            .http
            .get(&url)
            .header(
                reqwest::header::ACCEPT,
                "application/rss+xml, application/xml, text/xml, */*",
            )
            .query(&[("q", self.cfg.keywords.as_str())]);
        if let Some(loc) = self.cfg.location.as_deref().filter(|s| !s.is_empty()) {
            req = req.query(&[("l", loc)]);
        }
        if let Some(days) = self.cfg.fromage {
            let clamped = days.clamp(FROMAGE_MIN, FROMAGE_MAX);
            if clamped != days {
                warn!(
                    requested = days,
                    clamped,
                    "indeed_rss: fromage out of range; clamped to {FROMAGE_MIN}..={FROMAGE_MAX}",
                );
            }
            req = req.query(&[("fromage", clamped.to_string())]);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body = resp.text().await?;
        Ok(parse_feed(&body))
    }
}
