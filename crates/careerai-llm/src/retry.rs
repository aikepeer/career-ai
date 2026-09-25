//! Shared retry preset for LLM calls.
//!
//! Exponential backoff: 500ms → 8s, factor 2.0, with configurable retry
//! opportunities and jitter. Tuned to smooth over transient provider rate
//! limits without piling onto a persistent outage.

use std::time::Duration;

use backon::ExponentialBuilder;

#[must_use]
pub fn llm_backoff() -> ExponentialBuilder {
    llm_backoff_with_retries(3)
}

#[must_use]
pub fn llm_backoff_with_retries(max_retries: u32) -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(8))
        .with_factor(2.0)
        .with_max_times(max_retries as usize)
        .with_jitter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use backon::BackoffBuilder;

    #[test]
    fn llm_backoff_builds() {
        // Smoke test: ensure the preset builds with stable public API.
        let _ = llm_backoff();
    }

    #[test]
    fn retry_budget_counts_retry_opportunities() {
        for (retries, expected) in [(0, 0), (1, 1), (4, 4)] {
            let mut backoff = llm_backoff_with_retries(retries).build();
            assert_eq!(backoff.by_ref().count(), expected, "retries={retries}");
        }
    }
}
