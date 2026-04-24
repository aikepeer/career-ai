//! Layered config loader: embedded defaults → `config/default.yaml` →
//! `config/local.yaml` → `CAREERAI_*` env vars. Loaded once at startup.

use std::collections::HashMap;
use std::path::Path;

use config::{Config, ConfigError, Environment, File, FileFormat};
use serde::{Deserialize, Serialize};

/// Embedded fallback so the binary works without an `init`-scaffolded tree.
/// Source of truth for both this constant and the file written by `init` is
/// `templates/default.yaml`.
const EMBEDDED_DEFAULTS: &str = include_str!("templates/default.yaml");

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
pub struct SourcesConfig {
    #[serde(default)]
    pub greenhouse: CompaniesSource,
    #[serde(default)]
    pub lever: CompaniesSource,
    #[serde(default)]
    pub remotive: RemotiveSourceConfig,
    #[serde(default)]
    pub remoteok: ToggleSource,
    #[serde(default)]
    pub naukri: NaukriSourceConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompaniesSource {
    #[serde(default)]
    pub companies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotiveSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub category: Option<String>,
}

impl Default for RemotiveSourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            category: Some("software-dev".to_string()),
        }
    }
}

/// Naukri.com — India's largest job board. Uses the undocumented
/// `jobapi/v3/search` endpoint with `AppId`/`SystemId` headers. Defaults
/// to `enabled: false` because the endpoint is unofficial and Naukri
/// aggressively rate-limits unauthenticated callers; the user opts in
/// via `config/local.yaml` once they've decided to accept the trade-off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NaukriSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

impl Default for NaukriSourceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keywords: vec!["machine learning".into(), "llm".into(), "robotics".into()],
            location: Some("Delhi / NCR".into()),
            max_results: Some(20),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToggleSource {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ToggleSource {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_true() -> bool {
    true
}

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
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmitConfig {
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default)]
    pub per_source: HashMap<String, SubmitSource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmitSource {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub tailor_model: String,
    #[serde(default)]
    pub cover_letter_model: String,
    #[serde(default)]
    pub filter_model: String,
    #[serde(default)]
    pub parse_resume_model: String,
    /// Disk cache root for `careerai-llm::Cache`. Relative paths resolve
    /// against the workspace root at call time.
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Version tag baked into prompts + cache keys. Bump to invalidate all
    /// cached responses when prompt wording changes.
    #[serde(default = "default_prompt_version")]
    pub prompt_version: String,
    /// When true, `LlmRequest.cache_profile` is set so the provider impl
    /// attaches Anthropic prompt-cache metadata to the profile block.
    #[serde(default = "default_anthropic_prompt_cache")]
    pub anthropic_prompt_cache: bool,
}

fn default_cache_dir() -> String {
    "data/cache/llm".to_string()
}
fn default_max_retries() -> u32 {
    3
}
fn default_timeout_seconds() -> u64 {
    120
}
fn default_prompt_version() -> String {
    "tailor.v1".to_string()
}
fn default_anthropic_prompt_cache() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default)]
    pub cadence: HashMap<String, String>,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = CoreConfig::load(tmp.path()).unwrap();
        assert!(!cfg.user.locations.is_empty());
        assert!(cfg.user.locations.iter().any(|l| l.contains("Remote")));
        assert!(!cfg.domains.is_empty());
        assert!(cfg.matching.score_threshold > 0.0);
    }

    #[test]
    fn local_yaml_overrides_default() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg_dir = tmp.path().join("config");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("local.yaml"),
            "match:\n  embedding_model: stub-model\n  score_threshold: 0.42\n",
        )
        .unwrap();

        let cfg = CoreConfig::load(tmp.path()).unwrap();
        assert_eq!(cfg.matching.embedding_model, "stub-model");
        assert!((cfg.matching.score_threshold - 0.42).abs() < f32::EPSILON);
    }

    // Env-var override is wired via the `config` crate (prefix CAREERAI,
    // separator `__`) but not unit-tested here — std::env::set_var is
    // unsafe in modern Rust and the workspace forbids unsafe blocks.
}
