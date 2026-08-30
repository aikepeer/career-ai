//! Shared test fixtures for careerai-match unit tests.

use careerai_core::config::MatchConfig;

/// A `MatchConfig` with sane defaults for unit tests: no hard filters,
/// default tier boundaries, a zero score threshold.
#[must_use]
pub fn test_match_config() -> MatchConfig {
    MatchConfig {
        embedding_model: String::new(),
        score_threshold: 0.0,
        must_include_skills: vec![],
        notify_threshold: 0.05,
        company_blacklist: vec![],
        title_blacklist: vec![],
        location_blacklist: vec![],
        apply_once_at_company: false,
        tier_apply: 0.03,
        tier_apply_later: 0.02,
        tier_watch: 0.01,
    }
}
