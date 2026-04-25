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
    ///
    /// `None` when `min_seconds_between == 0` — i.e. the operator
    /// explicitly disabled the min-interval gate. We skip
    /// `until_ready()` entirely in that case rather than coercing 0
    /// to a 1-second floor (which would silently slow down tests and
    /// surprise operators who set 0 expecting "no gate").
    limiter: Option<Arc<GovLimiter>>,
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
    ///
    /// Returns a [`RatePermit`] that the caller MUST commit (via
    /// [`RatePermit::commit`]) after the work backed by the permit
    /// actually happened. Dropping the permit without commit refunds
    /// the day-cap slot — so an early failure after `acquire()` (e.g.
    /// missing credentials, browser launch error) does not consume
    /// the user's quota for the day. This keeps the day-cap measuring
    /// "real LinkedIn interactions" instead of "function calls."
    pub async fn acquire<'a>(
        &'a self,
        source: &str,
        policy: &RatePolicy,
    ) -> Result<RatePermit<'a>, RateLimitError> {
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
        // safety is restored by the `RatePermit` RAII handle below —
        // it decrements on Drop unless `commit()` is called after the
        // submission succeeds.
        let (limiter, today) = self.reserve_slot(source, policy, None).await?;

        // Permit owns the reservation. If we panic / cancel before
        // commit(), Drop releases the slot — no permanent day-cap leak.
        // The permit stores the day the slot was reserved on so a
        // refund decrements the *correct* bucket even if the day rolled
        // over before the permit was dropped.
        let mut permit = RatePermit::new(self, source.to_string(), today);

        // Min-interval wait outside the mutex (skipped entirely when
        // `min_seconds_between == 0` — caller wanted no gate).
        if let Some(lim) = limiter.as_ref() {
            lim.until_ready().await;
        }

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
                // Permit releases the reservation on its way out.
                return Err(RateLimitError::QuietHours { start, end });
            }
        }

        // Re-validate UTC day. If `until_ready()` + jitter pushed past
        // midnight, the reservation we made earlier was charged to the
        // OLD day's counter — and another caller may already have
        // observed the rollover and reset the bucket, in which case our
        // old-day slot was implicitly dropped. Either way, we need a
        // fresh slot reserved against TODAY before returning Ok.
        let now_day = UtcDay::now();
        if now_day != permit.reserved_day {
            // Reserve on the new day. The old-day reservation is moot;
            // when this permit drops without commit, refund() targets
            // permit.reserved_day — we update it below to point at the
            // new day so the refund hits the right bucket.
            //
            // If reserve_slot fails with DayCap on the new day, the
            // permit drops with reserved_day still pointing at the old
            // day. refund() will see entry.day != reserved_day and
            // no-op (the old bucket was already reset by another
            // caller, or by us inside reserve_slot below). Net effect:
            // DayCap is surfaced, no count is leaked.
            let _ = self.reserve_slot(source, policy, Some(now_day)).await?;
            permit.reserved_day = now_day;
        }

        // All gates passed. Hand the permit to the caller; the slot
        // stays reserved until the caller calls `permit.commit()` (work
        // succeeded) or drops it (refund).
        Ok(permit)
    }

    /// Reserve a day-cap slot under the lock. If `force_day` is
    /// `Some(d)`, the entry is migrated to `d` first and the slot is
    /// reserved against `d` regardless of the current `entry.day`. If
    /// `None`, the slot is reserved against today (with rollover from
    /// the cached day handled).
    ///
    /// Returns the per-source governor limiter handle (cloned out of
    /// the lock so the caller can `until_ready().await` without holding
    /// the mutex) and the day the slot was actually reserved on. The
    /// caller stores that day in the permit so refund decrements the
    /// right bucket on rollover.
    async fn reserve_slot(
        &self,
        source: &str,
        policy: &RatePolicy,
        force_day: Option<UtcDay>,
    ) -> Result<(Option<Arc<GovLimiter>>, UtcDay), RateLimitError> {
        let mut map = self.inner.lock().await;
        let entry = map
            .entry(source.to_string())
            .or_insert_with(|| build_per_source(policy));
        // Detect policy drift: if `policy.min_seconds_between` changed
        // since the cached `PerSource` was built (config reload,
        // different caller), rebuild the limiter so the governor quota
        // matches the request, not the first-ever-call's value.
        if entry.cached_min_seconds_between != policy.min_seconds_between {
            *entry = PerSource {
                day: entry.day,
                count_today: entry.count_today,
                ..build_per_source(policy)
            };
        }
        let target_day = force_day.unwrap_or_else(UtcDay::now);
        if entry.day != target_day {
            entry.day = target_day;
            entry.count_today = 0;
        }
        if entry.count_today >= policy.max_per_day {
            return Err(RateLimitError::DayCap {
                source_name: source.to_string(),
                cap: policy.max_per_day,
            });
        }
        entry.count_today = entry.count_today.saturating_add(1);
        Ok((entry.limiter.clone(), target_day))
    }

    /// Internal: refund a reservation made by `acquire`. Called from
    /// `RatePermit::drop` when the permit is dropped before commit.
    ///
    /// Day-aware: the permit stores the `UtcDay` its slot was reserved
    /// on. If `entry.day` no longer matches that day, the bucket has
    /// been rolled over to a new day — the old reservation was
    /// implicitly dropped by the rollover logic and there is nothing
    /// to refund. Without this check, a refund-after-rollover would
    /// decrement the WRONG day's counter and let a caller exceed
    /// max_per_day on the new day.
    ///
    /// Best-effort: `Drop` is sync, so we use `try_lock`. If the lock
    /// is contended (e.g. another task is mid-`acquire` for the same
    /// source) the refund is dropped silently — the slot stays
    /// reserved for the rest of the UTC day. We deliberately do NOT
    /// spawn a task to acquire later, because that would tie refund
    /// correctness to runtime liveness AND introduce a new ordering
    /// hazard (a refund firing after a fresh acquire on the next
    /// call). Day-cap stays a hard ceiling; under heavy contention
    /// the limiter may be slightly stricter than `max_per_day`, never
    /// looser.
    fn refund(&self, source: &str, reserved_day: UtcDay) {
        if let Ok(mut map) = self.inner.try_lock() {
            if let Some(entry) = map.get_mut(source) {
                if entry.day == reserved_day {
                    entry.count_today = entry.count_today.saturating_sub(1);
                }
                // else: bucket rolled over; old-day slot was already
                // released by the rollover. No-op.
            }
        }
    }
}

/// RAII permit for a reserved day-cap slot.
///
/// Returned by [`RateLimiter::acquire`]. Call [`RatePermit::commit`]
/// after the work that the permit was acquired for has actually
/// happened (e.g. a real interaction with the source's site). Dropping
/// without commit refunds the slot — so an early failure between
/// `acquire()` and the real interaction does NOT consume the user's
/// daily quota.
///
/// This keeps cap-as-hard-ceiling invariant under concurrent callers
/// (the bump under the lock makes count_today the authoritative count)
/// while restoring cancel-safety: dropping the permit mid-await
/// releases the reservation.
pub struct RatePermit<'a> {
    rl: &'a RateLimiter,
    source: String,
    /// UTC day the slot was reserved on. Carried so refund() can
    /// decrement the correct bucket if the day rolls over between
    /// `acquire()` and `drop`.
    reserved_day: UtcDay,
    committed: bool,
}

impl<'a> RatePermit<'a> {
    fn new(rl: &'a RateLimiter, source: String, reserved_day: UtcDay) -> Self {
        Self {
            rl,
            source,
            reserved_day,
            committed: false,
        }
    }

    /// Commit the reservation. Must be called once the work backed by
    /// this permit has actually happened. After commit, drop is a
    /// no-op — the day-cap slot stays consumed.
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl std::fmt::Debug for RatePermit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RatePermit")
            .field("source", &self.source)
            .field("reserved_day", &self.reserved_day)
            .field("committed", &self.committed)
            .finish()
    }
}

impl Drop for RatePermit<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.rl.refund(&self.source, self.reserved_day);
        }
    }
}

/// Build a fresh `PerSource` for the given policy.
///
/// `min_seconds_between == 0` is treated as "no min-interval gate" —
/// `limiter` is `None` and `acquire()` skips `until_ready()` entirely.
/// Earlier versions clamped 0 → 1 to satisfy `NonZeroU32`, but that
/// silently changed observable behavior (operators who set 0 expected
/// no gate; they got a 1s gate) and forced unit tests through real
/// wall-clock waits.
fn build_per_source(policy: &RatePolicy) -> PerSource {
    let limiter = if policy.min_seconds_between == 0 {
        None
    } else {
        // Safe: branch above ensures `min_seconds_between > 0`.
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

    /// Test policy: no min-interval, no jitter, no quiet hours.
    /// Lets unit tests exercise the day-cap + permit logic without
    /// real wall-clock waits.
    fn test_policy(max_per_day: u32) -> RatePolicy {
        RatePolicy {
            max_per_day,
            min_seconds_between: 0,
            jitter_seconds: 0,
            quiet_hours_utc: None,
        }
    }

    #[tokio::test]
    async fn day_cap_returns_error_when_exceeded() {
        let rl = RateLimiter::new();
        let policy = test_policy(2);
        rl.acquire("test", &policy).await.unwrap().commit();
        rl.acquire("test", &policy).await.unwrap().commit();
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

    #[tokio::test]
    async fn dropped_permit_refunds_day_cap_slot() {
        // Cap of 2. Acquire+drop (no commit) five times — every drop
        // must refund. Two committed acquires should still succeed
        // afterwards because the prior drops never consumed real quota.
        let rl = RateLimiter::new();
        let policy = test_policy(2);
        for _ in 0..5 {
            let _permit = rl.acquire("test", &policy).await.unwrap();
        }
        rl.acquire("test", &policy).await.unwrap().commit();
        rl.acquire("test", &policy).await.unwrap().commit();
        let err = rl.acquire("test", &policy).await.unwrap_err();
        assert!(matches!(err, RateLimitError::DayCap { .. }));
    }

    #[tokio::test]
    async fn committed_permit_consumes_day_cap_slot() {
        // Cap of 1. After commit, the slot is consumed permanently —
        // a second acquire must hit DayCap, no silent refund.
        let rl = RateLimiter::new();
        let policy = test_policy(1);
        rl.acquire("test", &policy).await.unwrap().commit();
        let err = rl.acquire("test", &policy).await.unwrap_err();
        assert!(matches!(err, RateLimitError::DayCap { .. }));
    }

    #[tokio::test]
    async fn refund_after_day_rollover_does_not_undercount_new_day() {
        // Regression for: a permit acquired on day N that drops on day
        // N+1 (after another caller already migrated the bucket) used
        // to decrement count_today on day N+1, undercounting the new
        // day's quota and letting callers exceed max_per_day.
        //
        // Construction: hand-poke the internal state to simulate the
        // rollover without waiting for real midnight. Acquire a permit
        // (reserved on the synthetic "yesterday"), then move the bucket
        // forward to today. Drop the permit. The new-day count must
        // remain whatever it was — refund must NOT touch today's
        // counter for a yesterday-day permit.
        let rl = RateLimiter::new();
        let policy = test_policy(3);

        // Acquire a permit. After this, entry.day = today, count_today
        // = 1, and permit.reserved_day = today.
        let permit = rl.acquire("test", &policy).await.unwrap();

        // Force a rollover: rewind permit.reserved_day to "yesterday"
        // in this RatePermit, leaving entry.day at today. Mirrors the
        // real-world sequence "permit reserved on day N, day rolled to
        // N+1, another caller reset entry to day N+1 with count_today=0,
        // then bumped count_today to 1 for itself."
        let yesterday = {
            let map = rl.inner.lock().await;
            let entry = map.get("test").unwrap();
            UtcDay {
                year: entry.day.year,
                ordinal: entry.day.ordinal.saturating_sub(1).max(1),
            }
        };
        // Commit the live permit so its drop doesn't refund the real
        // today bucket — we want to observe today's count from a
        // post-rollover refund of a STALE permit (constructed below),
        // not from this one.
        permit.commit();

        // "Another caller bumped count_today after the rollover" — the
        // half of the simulation where a parallel acquire takes a real
        // slot on today's bucket.
        let real_today_permit = rl.acquire("test", &policy).await.unwrap();
        let count_before_drop = {
            let map = rl.inner.lock().await;
            map.get("test").unwrap().count_today
        };

        // Now construct a stale permit pointing at yesterday and let it
        // drop. The refund must be a no-op — entry.day != reserved_day.
        {
            let _stale = RatePermit::new(&rl, "test".to_string(), yesterday);
        }

        let count_after_drop = {
            let map = rl.inner.lock().await;
            map.get("test").unwrap().count_today
        };
        assert_eq!(
            count_before_drop, count_after_drop,
            "stale-day refund must not decrement today's counter"
        );

        real_today_permit.commit();
    }

    #[tokio::test]
    async fn min_seconds_between_zero_skips_governor_wait() {
        // With min_seconds_between=0 the governor limiter is None and
        // acquire() never awaits a wall-clock delay. A burst of acquires
        // should complete under 50ms even for cap=10.
        let rl = RateLimiter::new();
        let policy = test_policy(10);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            rl.acquire("test", &policy).await.unwrap().commit();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 200,
            "acquire burst with min_seconds_between=0 took {}ms — governor wait was not skipped",
            elapsed.as_millis()
        );
    }
}
