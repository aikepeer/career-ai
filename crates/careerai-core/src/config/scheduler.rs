//! Scheduler configuration: per-source discovery cadences and optional
//! periodic apply-all sweep.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default)]
    pub cadence: HashMap<String, String>,
    /// Optional cron schedule that drives a periodic apply-all sweep over
    /// every shortlisted/rendered application. Distinct from `cadence`,
    /// which is keyed by source and only drives discover→match. When unset
    /// the daemon does not poll for submissions at all — operators apply
    /// manually via `careerai apply --auto-submit ...`. Even when set, the
    /// daemon hard-pins `auto_submit=false` for safety; see
    /// `careerai-scheduler::Scheduler::from_config`.
    #[serde(default)]
    pub submit_cadence: Option<String>,
    /// Optional cron schedule for periodic follow-up checking. When set,
    /// the daemon periodically calls `check_and_create_follow_ups` to
    /// create draft follow-up emails for submitted applications that
    /// haven't received a response. Mirrors the `submit_cadence` pattern.
    #[serde(default)]
    pub follow_up_cadence: Option<String>,
}
