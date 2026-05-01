//! Per-source rate-limit configuration (write-side).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RatesConfig {
    #[serde(flatten)]
    pub per_source: HashMap<String, SourceRate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceRate {
    #[serde(default)]
    pub max_per_day: u32,
    #[serde(default)]
    pub min_seconds_between: u32,
    #[serde(default)]
    pub jitter_seconds: u32,
    #[serde(default)]
    pub quiet_hours: Vec<u32>,
}
