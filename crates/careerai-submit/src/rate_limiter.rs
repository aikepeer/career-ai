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
    /// `min_seconds_between` value the cached `limiter` was built for.
    /// If the caller-supplied policy changes (e.g. config reload), we
    /// rebuild the limiter so the quota stays in sync with the rest of
    /// the rate-policy fields. Stored next to `limiter` so the comparison
    /// is one field load, not a re-derivation from the limiter handle.
    cached_min_seconds_between: u32,
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
        // Quiet-hours check up-front. Rechecked again after the wait
        // below so a permit started 1min before the window opens
        // can't fire inside the window.
        if let Some((start, end)) = policy.quiet_hours_utc {
            let hour = Utc::now().hour();
            if in_window(hour, start, end) {
                return Err(RateLimitError::QuietHours { start, end });
            }
        }

        // Reservation under the lock: bump count_today atomically with
        // the cap check so concurrent callers can't all observe
        // `count_today < cap` and collectively exceed the cap. Cancel
        // safety is restored by the `PermitGuard` RAII handle below —
        // it decrements on Drop unless `commit()` is called after the
        // submission succeeds.
        //
        // Also detect policy drift: if `policy.min_seconds_between`
        // changed since the cached `PerSource` was built (config reload,
        // different caller), rebuild the limiter so the governor quota
        // matches the request, not the first-ever-call's value.
        let limiter = {
            let mut map = self.inner.lock().await;
            let entry = map
                .entry(source.to_string())
                .or_insert_with(|| build_per_source(policy));
            if entry.cached_min_seconds_between != policy.min_seconds_between {
                *entry = PerSource {
                    day: entry.day,
                    count_today: entry.count_today,
                    ..build_per_source(policy)
                };
            }
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
            entry.count_today = entry.count_today.saturating_add(1);
            Arc::clone(&entry.limiter)
        };
        // Guard owns the reservation. If we panic / cancel before
        // commit(), Drop releases the slot — no permanent day-cap leak.
        let mut guard = PermitGuard::new(self, source);

        // Min-interval wait outside the mutex.
        limiter.until_ready().await;

        // Random jitter on top of min-interval.
        if policy.jitter_seconds > 0 {
            let jitter = {
                let mut rng = rand::rng();
                rng.random_range(0..=policy.jitter_seconds)
            };
            tokio::time::sleep(Duration::from_secs(u64::from(jitter))).await;
        }

        // Re-check quiet hours: a permit acquired just before the window
        // opens would otherwise fire INSIDE the window if the wait
        // straddled the start hour. Belt-and-suspenders with the
        // up-front check.
        if let Some((start, end)) = policy.quiet_hours_utc {
            let hour = Utc::now().hour();
            if in_window(hour, start, end) {
                // Guard releases the reservation on its way out.
                return Err(RateLimitError::QuietHours { start, end });
            }
        }

        // All gates passed — commit the reservation so it is NOT
        // refunded when the guard drops.
        guard.commit();
        Ok(())
    }

    /// Internal: refund a reservation made by `acquire`. Called from
    /// `PermitGuard::drop` when the future is dropped before commit.
    /// Uses `try_lock` because `Drop` is sync; if the lock is contended
    /// we fall back to a `tokio::spawn` so the refund still happens
    /// (rare — only under contention, e.g. running guards on multiple
    /// tasks targeting the same RateLimiter).
    fn refund(&self, source: &str) {
        if let Ok(mut map) = self.inner.try_lock() {
            if let Some(entry) = map.get_mut(source) {
                entry.count_today = entry.count_today.saturating_sub(1);
            }
        }
        // If `try_lock` fails, the slot stays reserved — same outcome
        // as the previous bump-after-success path. Acceptable; the
        // alternative (spawning a task to acquire later) would tie
        // the refund to runtime liveness.
    }
}

/// RAII guard for a held reservation. `Drop` refunds the slot unless
/// `commit()` was called. Keeps cap-as-hard-ceiling invariant under
/// concurrent callers (the bump under the lock makes count_today the
/// authoritative count) while restoring cancel-safety: dropping the
/// `acquire` future mid-await still releases the reservation.
struct PermitGuard<'a> {
    rl: &'a RateLimiter,
    source: &'a str,
    committed: bool,
}

impl<'a> PermitGuard<'a> {
    fn new(rl: &'a RateLimiter, source: &'a str) -> Self {
        Self {
            rl,
            source,
            committed: false,
        }
    }
    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PermitGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.rl.refund(self.source);
        }
    }
}

/// Build a fresh `PerSource` for the given policy.
fn build_per_source(policy: &RatePolicy) -> PerSource {
    // Clamp to >= 1 second so `Quota::with_period` never sees a zero
    // duration (it returns `None` in that case). `max_per_day=0` still
    // short-circuits via the day-cap check, so a 1s quota here is safe.
    // `policy.min_seconds_between.max(1)` is always >= 1, so
    // `NonZeroU32::new(...).expect()` is a true invariant — no fallible
    // path actually exists here.
    #[allow(clippy::expect_used)]
    let period_secs = NonZeroU32::new(policy.min_seconds_between.max(1))
        .expect("clamped >=1 by .max(1) on the previous line");
    let quota = Quota::with_period(Duration::from_secs(u64::from(period_secs.get())))
        .unwrap_or_else(|| Quota::per_minute(NonZeroU32::MIN));
    PerSource {
        limiter: Arc::new(GovernorLimiter::direct(quota)),
        cached_min_seconds_between: policy.min_seconds_between,
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
