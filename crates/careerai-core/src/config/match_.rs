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
    /// Companies never to shortlist (case-insensitive substring match on
    /// the listing company). Ported from AIHawk's `company_blacklist`.
    #[serde(default)]
    pub company_blacklist: Vec<String>,
    /// Title patterns never to shortlist. Supplements (does not replace)
    /// `rules.yaml::exclude_titles`. Ported from AIHawk's `title_blacklist`.
    #[serde(default)]
    pub title_blacklist: Vec<String>,
    /// Locations never to shortlist, applied on top of the `user.locations`
    /// allowlist (a listing rejected here may still pass the allowlist).
    /// Ported from AIHawk's `location_blacklist`.
    #[serde(default)]
    pub location_blacklist: Vec<String>,
    /// When true, a company with an existing application (any state) is
    /// never shortlisted again — one application per company, matching
    /// AIHawk's `apply_once_at_company`.
    #[serde(default)]
    pub apply_once_at_company: bool,
    /// Decision-tier boundaries (job_agentic APPLY/APPLY_LATER/WATCH/SKIP
    /// ladder, calibrated to this scorer's ~[0.0, 0.05] Jaccard scale).
    /// `tier_apply` >= score >= `tier_apply_later` => APPLY, etc.
    #[serde(default = "default_tier_apply")]
    pub tier_apply: f32,
    #[serde(default = "default_tier_apply_later")]
    pub tier_apply_later: f32,
    #[serde(default = "default_tier_watch")]
    pub tier_watch: f32,
}

fn default_tier_apply() -> f32 {
    0.03
}

fn default_tier_apply_later() -> f32 {
    0.02
}

fn default_tier_watch() -> f32 {
    0.01
}

fn default_match_notify_threshold() -> f32 {
    // Calibrated for the v1 `JaccardScorer` which produces scores in
    // ~[0.0, 0.05] on realistic profile/JD pairs. Bump together with
    // the score_threshold default in `templates/default.yaml` when an
    // embedding-based scorer lands.
    0.05
}
