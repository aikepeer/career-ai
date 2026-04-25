//! Per-source rate limiter for browser submitters.
//!
//! Composes three independent gates:
//!
//! 1. **Day-cap.** Hard ceiling on submissions per UTC-day per source.
//!    Counter persists in memory only — daemon restart re-zeros it,
//!    matching the existing pattern (M6 daemon will optionally
//!    persist).
//! 2. **Min-interval.** Minimum wall-clock seconds between consecutive
//!    permits, enforced by `governor`'s direct rate limiter. Prevents
//!    burst submissions even within the day-cap.
//! 3. **Quiet hours.** Skip-and-wait window in UTC. Configured as
//!    `[start_hour, end_hour)` in 24h format. Inside the window the
//!    limiter returns [`RateLimitError::QuietHours`] so callers can
//!    defer gracefully; outside it passes through. Wrap-across-midnight
//!    windows (e.g. `(22, 7)`) are supported.
//!
//! On top of (1)+(2), a small uniform-random jitter is added per
//! permit so two daemons on different schedules don't synchronize.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Timelike, Utc};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovernorLimiter,
};
use rand::Rng;
use tokio::sync::Mutex;

/// Per-source rate-limit policy. Mirrors the YAML shape under
/// `config.rates.<source>` so a `From<RatesConfig>` impl in M5b can
/// build these directly.
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
    // Field is named `source_name` (not `source`) because `thiserror`
    // treats a field literally named `source` as `#[source]`, which
    // requires an `Error` impl on its type.
    #[error("rate limit hit: {source_name} exceeded {cap}/day")]
    DayCap { source_name: String, cap: u32 },
    #[error("rate limit: in quiet hours [{start:02}:00, {end:02}:00) UTC")]
    QuietHours { start: u32, end: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UtcDay {
    year: i32,
    ordinal: u32,
}

impl UtcDay {
    fn now() -> Self {
        let now = Utc::now();
        Self::from_dt(&now)
    }
    fn from_dt(dt: &DateTime<Utc>) -> Self {
        Self {
            year: dt.year(),
            ordinal: dt.ordinal(),
        }
    }
}

type GovLimiter = GovernorLimiter<NotKeyed, InMemoryState, DefaultClock>;

struct PerSource {
    /// `governor` limiter enforcing min-interval. Quota is "1 cell per
    /// `min_seconds_between` seconds" — the `until_ready()` future
    /// awaits the next slot. Wrapped in `Arc` so we can clone the
    /// handle out of the HashMap and await without holding the mutex.
    limiter: Arc<GovLimiter>,
    /// Day counter (resets on UTC-day rollover).
    day: UtcDay,
    /// Submissions taken on `day` so far.
    count_today: u32,
}

/// Rate limiter shared across `Submitter` callers. `Arc<RateLimiter>`
/// is cheap; the inner state is `Mutex`-guarded because day-rollover
/// + count-bump must be atomic.
pub struct RateLimiter {
    inner: Mutex<HashMap<String, PerSource>>,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter").finish_non_exhaustive()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Acquire a permit for `source`. Awaits up to the min-interval +
    /// jitter; returns immediately on `DayCap` / `QuietHours` so the
    /// caller can mark the application Skipped without burning time.
    pub async fn acquire(&self, source: &str, policy: &RatePolicy) -> Result<(), RateLimitError> {
        // Quiet-hours check up-front: cheap and avoids waking governor.
        if let Some((start, end)) = policy.quiet_hours_utc {
            let hour = Utc::now().hour();
            if in_window(hour, start, end) {
                return Err(RateLimitError::QuietHours { start, end });
            }
        }

        // Day-cap CHECK (no bump) + clone governor handle. Bumping
        // before `until_ready().await` was the original posture, but
        // it leaks a day-cap slot when the caller drops the future
        // mid-await (panic, timeout, ctrl-c). The fix: only bump on
        // success, accepting a tiny race where two concurrent callers
        // could both observe count_today < cap before either bumps.
        // governor's direct limiter still serializes cell issuance, so
        // min-interval is enforced regardless. For M5a (auto_submit
        // false, click never fires) the race is harmless; for M5b/M6
        // it's an acceptable trade vs the cancel-leak alternative.
        let limiter = {
            let map = self.inner.lock().await;
            let mut map = map;
            let entry = map
                .entry(source.to_string())
                .or_insert_with(|| build_per_source(policy));
            let today = UtcDay::now();
            if entry.day != today {
                entry.day = today;
                entry.count_today = 0;
            }
            if entry.count_today >= policy.max_per_day {
                return Err(RateLimitError::DayCap {
                    source_name: source.to_string(),
                    cap: policy.max_per_day,
                });
            }
            Arc::clone(&entry.limiter)
        };

        // Min-interval wait outside the mutex.
        limiter.until_ready().await;

        // Random jitter on top, in addition to min-interval.
        if policy.jitter_seconds > 0 {
            let jitter = {
                let mut rng = rand::thread_rng();
                rng.gen_range(0..=policy.jitter_seconds)
            };
            tokio::time::sleep(Duration::from_secs(u64::from(jitter))).await;
        }

        // Bump on success only. Cancel-leak avoided.
        {
            let mut map = self.inner.lock().await;
            if let Some(entry) = map.get_mut(source) {
                let today = UtcDay::now();
                if entry.day != today {
                    entry.day = today;
                    entry.count_today = 0;
                }
                entry.count_today = entry.count_today.saturating_add(1);
            }
        }

        Ok(())
    }
}

/// Build a fresh `PerSource` for the given policy. Factored out so the
/// `or_insert_with` closure stays readable and the `Quota` fallback
/// logic has one home.
fn build_per_source(policy: &RatePolicy) -> PerSource {
    // Clamp to >= 1 second so `Quota::with_period` never sees a zero
    // duration (it returns `None` in that case). `max_per_day=0` still
    // short-circuits via the day-cap check, so a 1s quota here is safe.
    let period_secs = NonZeroU32::new(policy.min_seconds_between.max(1)).unwrap_or(NonZeroU32::MIN);
    let quota = Quota::with_period(Duration::from_secs(u64::from(period_secs.get())))
        .unwrap_or_else(|| Quota::per_minute(NonZeroU32::MIN));
    PerSource {
        limiter: Arc::new(GovernorLimiter::direct(quota)),
        day: UtcDay::now(),
        count_today: 0,
    }
}

/// True if `hour` falls inside the `[start, end)` UTC window. Wraps
/// across midnight (e.g. `quiet_hours = (22, 7)` means 22:00..07:00).
fn in_window(hour: u32, start: u32, end: u32) -> bool {
    if start <= end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn quiet_window_non_wrapping() {
        // 09:00..17:00
        assert!(!in_window(8, 9, 17));
        assert!(in_window(9, 9, 17));
        assert!(in_window(12, 9, 17));
        assert!(!in_window(17, 9, 17));
    }

    #[test]
    fn quiet_window_wrapping_midnight() {
        // 22:00..07:00 (overnight quiet hours)
        assert!(in_window(23, 22, 7));
        assert!(in_window(0, 22, 7));
        assert!(in_window(6, 22, 7));
        assert!(!in_window(7, 22, 7));
        assert!(!in_window(12, 22, 7));
        assert!(in_window(22, 22, 7));
    }

    #[tokio::test]
    async fn day_cap_returns_error_when_exceeded() {
        let rl = RateLimiter::new();
        let policy = RatePolicy {
            max_per_day: 2,
            min_seconds_between: 1,
            jitter_seconds: 0,
            quiet_hours_utc: None,
        };
        rl.acquire("test", &policy).await.unwrap();
        rl.acquire("test", &policy).await.unwrap();
        let err = rl.acquire("test", &policy).await.unwrap_err();
        match err {
            RateLimitError::DayCap { source_name, cap } => {
                assert_eq!(source_name, "test");
                assert_eq!(cap, 2);
            }
            RateLimitError::QuietHours { .. } => panic!("unexpected QuietHours"),
        }
    }

    #[tokio::test]
    async fn quiet_hours_returns_error_immediately() {
        // Set quiet hours covering ALL 24 hours — any current time is
        // inside. We're not testing the time logic here (covered above);
        // we're testing that the limiter surfaces QuietHours and
        // doesn't burn the day-cap counter.
        let rl = RateLimiter::new();
        let policy = RatePolicy {
            max_per_day: 5,
            min_seconds_between: 0,
            jitter_seconds: 0,
            quiet_hours_utc: Some((0, 24)),
        };
        let err = rl.acquire("test", &policy).await.unwrap_err();
        assert!(matches!(err, RateLimitError::QuietHours { .. }));
    }
}
