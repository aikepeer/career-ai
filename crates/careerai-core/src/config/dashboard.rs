//! Read-only HTTP dashboard configuration.
//!
//! Both fields are optional; the CLI applies built-in defaults
//! (port 8787, refresh 60s) when absent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub port: Option<u16>,
    pub refresh_seconds: Option<u32>,
}
