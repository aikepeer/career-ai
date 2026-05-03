use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Utc};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovernorLimiter,
};

/// Per-source rate-limit policy. Mirrors the YAML shape under
/// `config.rates.<source>`.
#[derive(Debug, Clone)]
pub struct RatePolicy {
    pub max_per_day: u32,
    pub min_seconds_between: u32,
    pub jitter_seconds: u32,
    /// Quiet-hours window `[start, end)` in UTC (24h). `None` disables
    /// the gate. `(22, 7)` means 22:00..07:00 UTC (overnight).
    pub quiet_hours_utc: Option<(u32, u32)>,
}

impl Default for RatePolicy {
    fn default() -> Self {
        Self {
            max_per_day: 50,
            min_seconds_between: 60,
            jitter_seconds: 30,
            quiet_hours_utc: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("rate limit hit: {source_name} exceeded {cap}/day")]
    DayCap { source_name: String, cap: u32 },
    #[error("rate limit: in quiet hours [{start:02}:00, {end:02}:00) UTC")]
    QuietHours { start: u32, end: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UtcDay {
    pub(crate) year: i32,
    pub(crate) ordinal: u32,
}

impl UtcDay {
    pub(crate) fn now() -> Self {
        let now = Utc::now();
        Self::from_dt(&now)
    }
    pub(crate) fn from_dt(dt: &DateTime<Utc>) -> Self {
        Self {
            year: dt.year(),
            ordinal: dt.ordinal(),
        }
    }
}

pub(crate) type GovLimiter = GovernorLimiter<NotKeyed, InMemoryState, DefaultClock>;

pub(crate) struct PerSource {
    pub(crate) limiter: Option<Arc<GovLimiter>>,
    pub(crate) cached_min_seconds_between: u32,
    pub(crate) day: UtcDay,
    pub(crate) count_today: u32,
}

pub(crate) fn build_per_source(policy: &RatePolicy) -> PerSource {
    let limiter = if policy.min_seconds_between == 0 {
        None
    } else {
        #[allow(clippy::expect_used)]
        let period_secs = NonZeroU32::new(policy.min_seconds_between)
            .expect("min_seconds_between > 0 verified by branch");
        let quota = Quota::with_period(Duration::from_secs(u64::from(period_secs.get())))
            .unwrap_or_else(|| Quota::per_minute(NonZeroU32::MIN));
        Some(Arc::new(GovernorLimiter::direct(quota)))
    };
    PerSource {
        limiter,
        cached_min_seconds_between: policy.min_seconds_between,
        day: UtcDay::now(),
        count_today: 0,
    }
}

/// True if `hour` falls inside the `[start, end)` UTC window.
pub(crate) fn in_window(hour: u32, start: u32, end: u32) -> bool {
    if start <= end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}
