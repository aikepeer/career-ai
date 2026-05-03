//! Per-source rate limiter for browser submitters.
//!
//! Composes three independent gates: day-cap, min-interval, quiet hours.
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod limiter;
mod permit;
#[cfg(test)]
mod tests;
pub(crate) mod types;

pub use limiter::RateLimiter;
pub use permit::RatePermit;
pub use types::{RateLimitError, RatePolicy};
