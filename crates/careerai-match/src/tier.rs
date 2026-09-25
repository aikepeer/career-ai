//! Decision tiers on match scores (ported from job_agentic's
//! APPLY / APPLY_LATER / WATCH / SKIP ladder).
//!
//! A shortlisted listing is more than "keep vs drop": the score tells the
//! operator how urgently to act. Tiers are pure classification over the
//! configured boundaries — no state, no I/O.

use careerai_core::config::MatchConfig;

/// How strongly a listing scored against the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Score >= `tier_apply` — tailor + apply now.
    Apply,
    /// Score >= `tier_apply_later` — strong but not urgent.
    ApplyLater,
    /// Score >= `tier_watch` — weak signal; watch, don't chase.
    Watch,
    /// Below every boundary — effectively a miss.
    Skip,
}

impl Tier {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::ApplyLater => "apply_later",
            Self::Watch => "watch",
            Self::Skip => "skip",
        }
    }
}

/// Classify `score` into a tier using `cfg`'s boundaries. A score below
/// `tier_watch` (or below every boundary) is `Skip`. Boundaries are
/// inclusive on the upper tiers.
#[must_use]
pub fn tier_for(score: f32, cfg: &MatchConfig) -> Tier {
    if score >= cfg.tier_apply {
        Tier::Apply
    } else if score >= cfg.tier_apply_later {
        Tier::ApplyLater
    } else if score >= cfg.tier_watch {
        Tier::Watch
    } else {
        Tier::Skip
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::tests_shared::test_match_config;

    fn cfg() -> MatchConfig {
        test_match_config()
    }

    #[test]
    fn classifies_into_four_tiers() {
        let c = cfg();
        assert_eq!(tier_for(0.05, &c), Tier::Apply);
        assert_eq!(tier_for(0.03, &c), Tier::Apply); // boundary inclusive
        assert_eq!(tier_for(0.025, &c), Tier::ApplyLater);
        assert_eq!(tier_for(0.02, &c), Tier::ApplyLater);
        assert_eq!(tier_for(0.015, &c), Tier::Watch);
        assert_eq!(tier_for(0.01, &c), Tier::Watch);
        assert_eq!(tier_for(0.005, &c), Tier::Skip);
        assert_eq!(tier_for(0.0, &c), Tier::Skip);
    }

    #[test]
    fn labels_match_job_agentic_vocabulary() {
        assert_eq!(Tier::Apply.label(), "apply");
        assert_eq!(Tier::ApplyLater.label(), "apply_later");
        assert_eq!(Tier::Watch.label(), "watch");
        assert_eq!(Tier::Skip.label(), "skip");
    }

    #[test]
    fn custom_boundaries_are_honored() {
        let mut c = cfg();
        c.tier_apply = 0.9;
        c.tier_apply_later = 0.5;
        c.tier_watch = 0.1;
        assert_eq!(tier_for(0.95, &c), Tier::Apply);
        assert_eq!(tier_for(0.7, &c), Tier::ApplyLater);
        assert_eq!(tier_for(0.2, &c), Tier::Watch);
        assert_eq!(tier_for(0.05, &c), Tier::Skip);
    }

    #[test]
    fn inverted_boundaries_do_not_panic() {
        let mut c = cfg();
        c.tier_apply = 0.0;
        c.tier_apply_later = 0.0;
        c.tier_watch = 0.0;
        assert_eq!(tier_for(0.0, &c), Tier::Apply);
    }
}
