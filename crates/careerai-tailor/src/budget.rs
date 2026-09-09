//! F07 budget enforcement gate.
//!
//! Pure decision function: given the current UTC-day spend + call count
//! and the operator's configured caps, decide whether the tailor should
//! fall back to local-only mode for this listing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_core::config::LlmConfig;

/// Outcome of the budget check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetDecision {
    /// Within budget — proceed with the LLM call.
    Allow,
    /// Daily spend cap exceeded — fall back to local-only.
    SpendCapExceeded,
    /// Daily call cap reached — fall back to local-only.
    CallCapReached,
}

/// Decide whether an LLM call is allowed under the configured daily caps.
///
/// `daily_cost_usd` is the cumulative spend for the current UTC day.
/// `daily_calls` is the cumulative call count for the current UTC day.
/// Both come from `CostTracker::daily_cost_from_disk` /
/// `daily_call_count_from_disk`.
#[must_use]
pub fn decide(cfg: &LlmConfig, daily_cost_usd: f64, daily_calls: u64) -> BudgetDecision {
    if let Some(cap) = cfg.max_daily_cost_usd {
        if daily_cost_usd >= cap {
            return BudgetDecision::SpendCapExceeded;
        }
    }
    if let Some(cap) = cfg.max_daily_calls {
        if daily_calls >= cap {
            return BudgetDecision::CallCapReached;
        }
    }
    BudgetDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(cost: Option<f64>, calls: Option<u64>) -> LlmConfig {
        LlmConfig {
            max_daily_cost_usd: cost,
            max_daily_calls: calls,
            ..Default::default()
        }
    }

    #[test]
    fn allows_when_no_caps_set() {
        let c = cfg(None, None);
        assert_eq!(decide(&c, 999.0, 999), BudgetDecision::Allow);
    }

    #[test]
    fn allows_when_under_both_caps() {
        let c = cfg(Some(1.0), Some(100));
        assert_eq!(decide(&c, 0.5, 50), BudgetDecision::Allow);
    }

    #[test]
    fn blocks_when_spend_cap_exceeded() {
        let c = cfg(Some(1.0), None);
        assert_eq!(decide(&c, 1.0, 0), BudgetDecision::SpendCapExceeded);
        assert_eq!(decide(&c, 1.5, 0), BudgetDecision::SpendCapExceeded);
    }

    #[test]
    fn blocks_when_call_cap_reached() {
        let c = cfg(None, Some(10));
        assert_eq!(decide(&c, 0.0, 10), BudgetDecision::CallCapReached);
        assert_eq!(decide(&c, 0.0, 15), BudgetDecision::CallCapReached);
    }

    #[test]
    fn spend_cap_takes_precedence_over_call_cap() {
        // If both are exceeded, spend cap wins (it's checked first).
        let c = cfg(Some(1.0), Some(10));
        assert_eq!(decide(&c, 2.0, 20), BudgetDecision::SpendCapExceeded);
    }

    #[test]
    fn allows_at_boundary_under_cap() {
        let c = cfg(Some(1.0), Some(10));
        // Just under both caps.
        assert_eq!(decide(&c, 0.99, 9), BudgetDecision::Allow);
    }
}
