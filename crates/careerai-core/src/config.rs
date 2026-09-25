//! Layered config loader: embedded defaults → XDG config
//! (`$XDG_CONFIG_HOME/career-ai/default.yaml` and `local.yaml`) →
//! `CAREERAI_*` env vars. Explicit `CAREERAI_ROOT` keeps project-local config.
//!
//! Section configs are split into submodules under `config/` to keep
//! each file under the project's 300-LOC cap. The umbrella [`CoreConfig`]
//! lives here and re-exports every section type so existing callers can
//! continue importing `careerai_core::config::FooConfig` without churn.

use std::path::Path;

use config::{Config, ConfigError, Environment, File, FileFormat};
use serde::{Deserialize, Serialize};

mod cluster;
mod dashboard;
mod llm;
mod match_;
mod rates;
mod render;
mod scheduler;
mod sources;
mod submit;
#[cfg(test)]
mod tests;
pub use cluster::ClusterConfig;

pub use dashboard::DashboardConfig;
pub use render::RenderConfig;
pub use scheduler::SchedulerConfig;

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
    pub cluster: ClusterConfig,
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

impl CoreConfig {
    /// Load layered configuration from the resolved XDG config directory.
    ///
    /// An explicit project root remains supported for isolated workspaces;
    /// normal invocations read `XDG_CONFIG_HOME/career-ai`.
    pub fn load(root: &Path) -> Result<Self, ConfigError> {
        let config_dir = crate::paths::config_dir_for_root(root);
        let default_path = config_dir.join("default.yaml");
        let local_path = config_dir.join("local.yaml");

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
