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
}
