//! Profile + matching configuration: who the user is, what domains
//! they're targeting, and how listings are scored against the profile.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub locations: Vec<String>,
    pub timezone: String,
    #[serde(default)]
    pub work_auth: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Domain {
    pub name: String,
    #[serde(default)]
    pub keywords_any: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchConfig {
    pub embedding_model: String,
    pub score_threshold: f32,
    /// Hard filter applied BEFORE scoring. A listing must contain at least
    /// one of these tokens (case-insensitive substring match) anywhere in
    /// its title, description, or normalized skill set, or it transitions
    /// directly to `filtered_out`. Empty = no hard filter (default
    /// behavior matches pre-W1 builds).
    #[serde(default)]
    pub must_include_skills: Vec<String>,
    /// Score threshold above which a shortlisted listing fires a
    /// `HighScoreMatch` notification through `careerai-notify`. Set
    /// strictly higher than `score_threshold` so only the most
    /// promising listings trigger pings — same-day applies tend to
    /// convert at this band.
    #[serde(default = "default_match_notify_threshold")]
    pub notify_threshold: f32,
}

fn default_match_notify_threshold() -> f32 {
    // Calibrated for the v1 `JaccardScorer` which produces scores in
    // ~[0.0, 0.05] on realistic profile/JD pairs. Bump together with
    // the score_threshold default in `templates/default.yaml` when an
    // embedding-based scorer lands.
    0.05
}
