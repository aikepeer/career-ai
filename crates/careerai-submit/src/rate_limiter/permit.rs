use super::limiter::RateLimiter;
use crate::rate_limiter::types::UtcDay;

/// RAII permit for a reserved day-cap slot.
///
/// Returned by [`RateLimiter::acquire`]. Call [`RatePermit::commit`]
/// after the work that the permit was acquired for has actually
/// happened. Dropping without commit refunds the slot.
pub struct RatePermit<'a> {
    rl: &'a RateLimiter,
    source: String,
    pub(crate) reserved_day: UtcDay,
    committed: bool,
}

impl<'a> RatePermit<'a> {
    pub(crate) fn new(rl: &'a RateLimiter, source: String, reserved_day: UtcDay) -> Self {
        Self {
            rl,
            source,
            reserved_day,
            committed: false,
        }
    }

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
