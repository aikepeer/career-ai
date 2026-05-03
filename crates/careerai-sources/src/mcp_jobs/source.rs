//! MCP source driver: `McpJobsSource` struct, rate limiter, and `Source`
//! trait implementation.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use careerai_core::config::McpSourceConfig;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use rmcp::ClientHandler;

use crate::base::{RawListing, Source, SourceError};

use super::discover::run_call;

/// Wall-clock cap for `initialize` + `tools/list` + the tool call.
/// Community MCP servers occasionally hang on slow upstream APIs; a
/// hard ceiling keeps a misbehaving source from blocking the cron tick.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(45);

/// Tool-name aliases tried in order. The first matching tool advertised
/// by the server is used.
pub(crate) const TOOL_NAME_ALIASES: &[&str] = &[
    "search_jobs",
    "discover_jobs",
    "find_jobs",
    "list_jobs",
    "jobs.search",
];

/// Default empty client handler. The default impl already advertises
/// sensible `ClientInfo`; we don't need elicitation, sampling, or
/// roots.
#[derive(Default, Clone, Debug)]
pub(crate) struct ProbeClient;

impl ClientHandler for ProbeClient {}

/// Per-instance read-side rate limiter. We deliberately do NOT pull
/// `careerai-submit::RateLimiter` here: that crate is downstream of
/// `careerai-sources` (sources never depends on submit, see
/// `CLAUDE.md`'s crate-boundary table) and its day-cap / quiet-hours
/// machinery is sized for write-side traffic. Read-side discovery
/// gets a simple `governor` token-bucket sized in calls-per-minute,
/// which is enough to keep us inside community RapidAPI free-tier
/// limits without inverting the dep graph.
type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

/// Per-name interner for `(name_static, rate_limiter)` pairs.
///
/// `build_sources()` is invoked on every cron tick, which means
/// `McpJobsSource::new` is called repeatedly with the same `cfg.name`
/// over the daemon's lifetime. Without this cache:
///   1. Every call would `Box::leak` a fresh copy of the source name —
///      one slow leak per tick, per source. Bounded but unbounded over
///      uptime; pre-fix doc comment claimed otherwise and was wrong.
///   2. Every call would build a new `RateLimiter` with a full bucket,
///      so the per-minute cap never fired in practice.
///
/// With the cache: one allocation per distinct source name, and the
/// token-bucket state is shared across reconstructions so the cap is
/// honored.
type InternedState = (&'static str, Option<Arc<ReadRateLimiter>>);
type InternMap = std::sync::Mutex<std::collections::HashMap<String, InternedState>>;
static SOURCE_STATE: std::sync::OnceLock<InternMap> = std::sync::OnceLock::new();

pub(crate) fn intern_source_state(name: &str, rate_per_minute: u32) -> InternedState {
    let map = SOURCE_STATE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // `lock()` only fails if a previous holder panicked. The state we
    // keep is just `(static str, Arc<RateLimiter>)`; recovery is safe.
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = guard.get(name) {
        return entry.clone();
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let rl = NonZeroU32::new(rate_per_minute)
        .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
    guard.insert(name.to_owned(), (leaked, rl.clone()));
    (leaked, rl)
}

#[derive(Debug)]
pub struct McpJobsSource {
    cfg: McpSourceConfig,
    /// `&'static str` produced via a name-keyed interner backed by
    /// `Box::leak`. Constructing two `McpJobsSource` instances with the
    /// same `cfg.name` returns the same `&'static str` (one allocation
    /// per distinct source name, for the lifetime of the process). The
    /// scheduler invokes `build_sources()` on every cron tick — without
    /// the cache that would leak a fresh string every tick.
    pub(crate) name_static: &'static str,
    /// Shared with all other `McpJobsSource` instances that have the
    /// same `cfg.name`. The token-bucket state lives in the `Arc`, so a
    /// new construction (e.g. on the next cron tick via
    /// `build_sources()`) does not reset the bucket — the per-minute
    /// cap is honored across ticks.
    pub(crate) rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl McpJobsSource {
    #[must_use]
    pub fn new(cfg: McpSourceConfig) -> Self {
        let (name_static, rate_limiter) = intern_source_state(&cfg.name, cfg.rate_per_minute);
        Self {
            cfg,
            name_static,
            rate_limiter,
        }
    }

    pub(crate) fn cfg(&self) -> &McpSourceConfig {
        &self.cfg
    }
}

#[async_trait]
impl Source for McpJobsSource {
    fn name(&self) -> &'static str {
        self.name_static
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        if let Some(rl) = &self.rate_limiter {
            // Read-side permit. `until_ready` waits if the bucket is
            // empty, but never blocks longer than `1/rate_per_minute`
            // — bounded by the per-minute quota — so the daemon tick
            // is not held hostage by a misconfigured cap.
            rl.until_ready().await;
        }

        let result = tokio::time::timeout(CALL_TIMEOUT, run_call(self))
            .await
            .map_err(|_| {
                SourceError::Parse(format!(
                    "mcp source `{}` timed out after {}s",
                    self.name_static,
                    CALL_TIMEOUT.as_secs(),
                ))
            })??;
        Ok(result)
    }
}
