//! Configuration for optional shortlisted-JD clustering.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Clustering is opt-in until quality is measured on the operator's jobs.
    #[serde(default)]
    pub enabled: bool,
    /// Minimum token-Jaccard similarity for two JDs to share a cluster.
    #[serde(default = "default_threshold")]
    pub threshold: f32,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: default_threshold(),
        }
    }
}

fn default_threshold() -> f32 {
    0.85
}
