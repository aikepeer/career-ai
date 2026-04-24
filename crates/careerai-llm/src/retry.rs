//! Shared retry preset for LLM calls.
//!
//! Exponential backoff: 500ms → 8s, factor 2.0, up to 3 attempts, jitter
//! enabled. Tuned to smooth over transient provider rate limits without
//! piling onto a persistent outage.

use std::time::Duration;

use backon::ExponentialBuilder;

#[must_use]
pub fn llm_backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(8))
        .with_factor(2.0)
        .with_max_times(3)
        .with_jitter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_backoff_builds() {
        // Smoke test: ensure the preset builds with stable public API.
        let _ = llm_backoff();
    }
}
