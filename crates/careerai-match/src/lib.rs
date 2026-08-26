//! Listing ↔ profile matcher.
//!
//! Pipeline: hard filters (`filters::classify`) → score each keeper
//! (`score::Scorer`) → rank + threshold split (`rank`). Embedding-based
//! scoring will land as a new `Scorer` impl; nothing else needs to change
//! when it does.

pub mod bullet_score;
pub mod error;
pub mod filters;
pub mod rank;
pub mod rules;
pub mod score;

pub use bullet_score::{BulletScorer, JaccardBulletScorer};
pub use error::{MatchError, Result};
pub use filters::{classify, Decision};
pub use rank::{rank_all, score_histogram, split_at_threshold, Scored};
pub use rules::FilterRules;
pub use score::{flatten_profile, JaccardScorer, Scorer};
