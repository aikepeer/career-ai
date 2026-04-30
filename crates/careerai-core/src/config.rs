//! Layered config loader: embedded defaults → `config/default.yaml` →
//! `config/local.yaml` → `CAREERAI_*` env vars. Loaded once at startup.
//!
//! Section configs are split into submodules under `config/` to keep
//! each file under the project's 300-LOC cap. The umbrella [`CoreConfig`]
//! lives here and re-exports every section type so existing callers can
//! continue importing `careerai_core::config::FooConfig` without churn.

use std::collections::HashMap;
use std::path::Path;

use config::{Config, ConfigError, Environment, File, FileFormat};
use serde::{Deserialize, Serialize};

mod llm;
mod match_;
mod rates;
mod sources;
mod submit;
#[cfg(test)]
mod tests;

pub use llm::{BackendChoice, LlmConfig};
pub use match_::{Domain, MatchConfig, UserConfig};
pub use rates::{RatesConfig, SourceRate};
pub use sources::{
    CompaniesSource, IndeedRssSourceConfig, LinkedinBrowserFilters, LinkedinBrowserSourceConfig,
    McpQueryConfig, McpSourceConfig, McpTransportConfig, NaukriSourceConfig, RemotiveSourceConfig,
    SourcesConfig, ToggleSource,
};
pub use submit::{LinkedinSubmitConfig, NaukriSubmitConfig, SubmitConfig, SubmitSource};

/// Embedded fallback so the binary works without an `init`-scaffolded tree.
/// Source of truth for both this constant and the file written by `init` is
/// `templates/default.yaml`. Public so downstream crates (notably the
/// `careerai-sources` company-sync tests) can hydrate a default
/// `CoreConfig` without depending on the on-disk layout.
pub const EMBEDDED_DEFAULTS: &str = include_str!("templates/default.yaml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub user: UserConfig,
    pub domains: Vec<Domain>,
    #[serde(rename = "match")]
    pub matching: MatchConfig,
    #[serde(default)]
    pub rates: RatesConfig,
    #[serde(default)]
    pub submit: SubmitConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub render: RenderConfig,
    /// Notification pipeline (Slack / Telegram / email / ntfy). Defaults
    /// to `min_severity = warning` with no channels enabled, so a fresh
    /// install never tries to talk to an external endpoint until the
    /// operator configures one.
    #[serde(default)]
    pub notify: careerai_notify::NotifyConfig,
    /// Read-only HTTP dashboard. Both fields are optional; the CLI
    /// applies built-in defaults (port 8787, refresh 60s) when absent.
    #[serde(default)]
    pub dashboard: DashboardConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub port: Option<u16>,
    pub refresh_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    pub artifacts_dir: std::path::PathBuf,
    pub pandoc_bin: Option<std::path::PathBuf>,
    pub pdf_engine: String,
    pub timeout_seconds: u64,
    pub keep_intermediate_markdown: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            artifacts_dir: std::path::PathBuf::from("artifacts"),
            pandoc_bin: None,
            pdf_engine: "weasyprint".into(),
            timeout_seconds: 60,
            keep_intermediate_markdown: true,
        }
    }
}

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

impl CoreConfig {
    /// Load with the standard layering relative to `root`. Missing
    /// `default.yaml` and `local.yaml` are tolerated; embedded defaults fill
    /// any gap.
    pub fn load(root: &Path) -> Result<Self, ConfigError> {
        let default_path = root.join("config").join("default.yaml");
        let local_path = root.join("config").join("local.yaml");

        let mut builder =
            Config::builder().add_source(File::from_str(EMBEDDED_DEFAULTS, FileFormat::Yaml));
        if default_path.exists() {
            builder = builder.add_source(File::from(default_path));
        }
        if local_path.exists() {
            builder = builder.add_source(File::from(local_path));
        }
        builder = builder.add_source(
            Environment::with_prefix("CAREERAI")
                .separator("__")
                .try_parsing(true),
        );
        builder.build()?.try_deserialize()
    }
}
