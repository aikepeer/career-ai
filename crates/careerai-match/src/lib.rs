//! Listing ↔ profile matcher.
//!
//! Pipeline: hard filters (`filters::classify`) → score each keeper
//! (`score::Scorer`) → rank + threshold split (`rank`). Embedding-based
//! scoring will land as a new `Scorer` impl; nothing else needs to change
//! when it does.

pub mod bullet_score;
pub mod eligibility;
pub mod error;
pub mod filters;
pub mod legitimacy;
pub mod profile_analyzer;
pub mod rank;
pub mod rules;
pub mod salary;
pub mod score;
pub mod skill_gap;
pub mod tier;
pub mod upskill;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod tests_shared;

pub use bullet_score::{BulletScorer, JaccardBulletScorer};
pub use eligibility::{check_eligibility, AuthProfile, EligibilityResult};
pub use error::{MatchError, Result};
pub use filters::{classify, Decision};
pub use legitimacy::{assess_legitimacy, LegitimacyScore};
pub use profile_analyzer::{analyze_profile, Finding, ProfileReport, ProfileStats, Severity};
pub use rank::{rank_all, score_histogram, split_at_threshold, Scored};
pub use rules::FilterRules;
pub use salary::{extract_salary_range, SalaryRange};
pub use score::{flatten_profile, match_breakdown, JaccardScorer, MatchBreakdown, Scorer};
pub use skill_gap::{analyze_skill_gaps, SkillGapEntry, SkillGapReport as JDSkillGapReport};
pub use tier::{tier_for, Tier};
pub use upskill::analyse_skill_gap;
