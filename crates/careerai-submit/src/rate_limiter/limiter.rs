use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Timelike;
use rand::Rng;
use tokio::sync::Mutex;
use tracing::warn;

use super::permit::RatePermit;
use super::types::{
    build_per_source, in_window, GovLimiter, PerSource, RateLimitError, RatePolicy, UtcDay,
};

/// Rate limiter shared across `Submitter` callers.
pub struct RateLimiter {
    pub(crate) inner: Mutex<HashMap<String, PerSource>>,
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

    /// Acquire a permit for `source`.
    pub async fn acquire<'a>(
        &'a self,
        source: &str,
        policy: &RatePolicy,
    ) -> Result<RatePermit<'a>, RateLimitError> {
        if let Some((start, end)) = policy.quiet_hours_utc {
            let hour = chrono::Utc::now().hour();
            if in_window(hour, start, end) {
                return Err(RateLimitError::QuietHours { start, end });
            }
        }

        let (limiter, today) = self.reserve_slot(source, policy, None).await?;

        let mut permit = RatePermit::new(self, source.to_string(), today);

        if let Some(lim) = limiter.as_ref() {
            lim.until_ready().await;
        }

        if policy.jitter_seconds > 0 {
            let jitter = {
                let mut rng = rand::rng();
                rng.random_range(0..=policy.jitter_seconds)
            };
            tokio::time::sleep(Duration::from_secs(u64::from(jitter))).await;
        }

        if let Some((start, end)) = policy.quiet_hours_utc {
            let hour = chrono::Utc::now().hour();
            if in_window(hour, start, end) {
                return Err(RateLimitError::QuietHours { start, end });
            }
        }

        let now_day = UtcDay::now();
        if now_day != permit.reserved_day {
            let _ = self.reserve_slot(source, policy, Some(now_day)).await?;
            permit.reserved_day = now_day;
        }

        Ok(permit)
    }

    /// Reserve a day-cap slot under the lock.
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

    /// Refund a reservation made by `acquire`. Best-effort; logs a
    /// warning when the lock is contended so operators can detect
    /// sustained rate-limit drift.
    pub(crate) fn refund(&self, source: &str, reserved_day: UtcDay) {
        match self.inner.try_lock() {
            Ok(mut map) => {
                if let Some(entry) = map.get_mut(source) {
                    if entry.day == reserved_day {
                        entry.count_today = entry.count_today.saturating_sub(1);
                    }
                }
            }
            Err(_) => {
                warn!(
                    target = "rate_limiter",
                    source = %source,
                    "rate-limit refund dropped: lock contended; day-cap counter may drift slightly"
                );
            }
        }
    }
}
