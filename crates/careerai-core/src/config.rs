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
    /// Hard filter applied BEFORE scoring. A listing must contain at least
    /// one of these tokens (case-insensitive substring match) anywhere in
    /// its title, description, or normalized skill set, or it transitions
    /// directly to `filtered_out`. Empty = no hard filter (default
    /// behavior matches pre-W1 builds).
    #[serde(default)]
    pub must_include_skills: Vec<String>,
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
#[serde(default)]
pub struct SubmitConfig {
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default)]
    pub per_source: HashMap<String, SubmitSource>,
    /// LinkedIn browser submitter knobs (M5a). Only consulted when the
    /// CLI is built with `--features browser`. Additive and `#[serde(default)]`
    /// so pre-M5a `local.yaml` files keep parsing.
    #[serde(default)]
    pub linkedin: LinkedinSubmitConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmitSource {
    #[serde(default)]
    pub enabled: bool,
}

/// LinkedIn-specific browser submitter configuration. Loaded from
/// `submit.linkedin.*` in `config/{default,local}.yaml`. Mirrors the
/// runtime `careerai_submit::LinkedinConfig` one-for-one so the
/// wire-up in `LinkedinConfig::from_core` is a trivial field copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkedinSubmitConfig {
    /// Where the submitter writes pre-submit screenshots. Created on
    /// demand at submit time; relative paths resolve against the
    /// process cwd (typically the repo root).
    pub screenshots_dir: std::path::PathBuf,
    /// Optional Chromium user-agent override. `None` → use
    /// `BrowserSessionConfig` default.
    pub user_agent: Option<String>,
    /// Headless Chromium. Default `true`.
    pub headless: bool,
    /// Rate-limit policy — max submissions per UTC day.
    pub max_per_day: u32,
    /// Minimum seconds between two submissions.
    pub min_seconds_between: u32,
    /// Uniform random jitter (seconds) added after the min-interval
    /// wait to avoid burst fingerprinting.
    pub jitter_seconds: u32,
    /// `[start_hour_utc, end_hour_utc)` window when the limiter refuses
    /// permits. Wraps across midnight (e.g. `[19, 1)` = late IST night).
    /// Both values are validated to be in `0..=23` (start) and `0..=24`
    /// (end inclusive) at config load via `LinkedinSubmitConfig::validated`;
    /// out-of-range values emit a warning and clamp to the default
    /// rather than silently disabling the gate.
    pub quiet_hours_utc: Option<(u32, u32)>,
    /// Per-action selector / click timeout in seconds.
    pub action_timeout_seconds: u64,
    /// Inner kill-switch enforced by `LinkedinSubmitter::run_session`:
    /// even with `auto_submit=true` AND
    /// `per_source.linkedin.enabled=true` AND a valid `li_at` cookie,
    /// the final "Submit application" click stays suppressed unless
    /// this flag is explicitly true. M5a never flips it (the submitter
    /// returns `SourceDisabled` after the audit screenshot); M5b will.
    /// Two locks must be lifted together to ship the click — this flag
    /// AND the M5b code that replaces the inner `SourceDisabled`
    /// return with the actual `element.click()`.
    pub allow_submit_click: bool,
    /// When `true` (default), the daemon never opens a LinkedIn browser
    /// session and never clicks Submit autonomously. After rendering,
    /// LinkedIn applications transition to `Drafted` instead. Operators
    /// confirm submits one at a time via `careerai review`. Flip to
    /// `false` only after weighing the account-restriction risk.
    #[serde(default = "default_interactive_only")]
    pub interactive_only: bool,
}

fn default_interactive_only() -> bool {
    true
}

impl Default for LinkedinSubmitConfig {
    fn default() -> Self {
        Self {
            screenshots_dir: std::path::PathBuf::from("artifacts/screenshots/linkedin"),
            user_agent: None,
            headless: true,
            max_per_day: 10,
            min_seconds_between: 120,
            jitter_seconds: 60,
            // 19:00 UTC = 00:30 IST; runs through 01:00 UTC, covering
            // late IST night when no human is reviewing submissions.
            quiet_hours_utc: Some((19, 1)),
            action_timeout_seconds: 20,
            // Defense-in-depth default OFF — see field doc.
            allow_submit_click: false,
            interactive_only: true,
        }
    }
}

impl LinkedinSubmitConfig {
    /// Range-validate `quiet_hours_utc`. Out-of-range values silently
    /// disabled the quiet-hours gate before, undermining the safety
    /// posture; now we clamp them to the default window and log a
    /// warning. Called from `from_core` in `careerai-submit::linkedin`
    /// so every consumer gets a vetted policy.
    ///
    /// Valid range:
    /// - `start` in `0..=23`, `end` in `0..=24`.
    /// - `start != end` (equal pair is ambiguous: "always quiet" vs
    ///   "never quiet" — either is a footgun, so reject).
    /// - `(0, 24)` is also rejected because `in_window` treats it as
    ///   the wrap-across-midnight pair `[0:00, 24:00)` covering every
    ///   hour — i.e. always quiet, indistinguishable from leaving the
    ///   submitter disabled. Operators who want "never submit" should
    ///   set `submit.linkedin.enabled = false` instead.
    /// - To disable the quiet-hours gate entirely, set
    ///   `quiet_hours_utc: null` (None).
    #[must_use]
    pub fn validated(mut self) -> Self {
        if let Some((start, end)) = self.quiet_hours_utc {
            let start_ok = start <= 23;
            let end_ok = end <= 24;
            let distinct = start != end;
            // (0, 24) is a degenerate full-coverage wrap — same as
            // "always quiet". Surface it as out-of-range so operators
            // notice they probably wanted `null` or
            // `submit.linkedin.enabled = false`.
            let not_full_coverage = !(start == 0 && end == 24);
            if !(start_ok && end_ok && distinct && not_full_coverage) {
                tracing::warn!(
                    target: "config",
                    submit_linkedin_quiet_hours = ?(start, end),
                    "out-of-range or full-coverage quiet_hours_utc; clamping to default (19, 1) — \
                     valid range is start in 0..=23, end in 0..=24, start != end, and (0, 24) is reserved \
                     (use `quiet_hours_utc: null` to disable the gate, or `submit.linkedin.enabled = false`)"
                );
                self.quiet_hours_utc = Some((19, 1));
            }
        }
        self
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn match_config_must_include_skills_defaults_empty() {
        let yaml = "embedding_model: \"x\"\nscore_threshold: 0.5";
        let cfg: MatchConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.must_include_skills.is_empty());
    }

    #[test]
    fn match_config_parses_must_include_skills() {
        let yaml =
            "embedding_model: \"x\"\nscore_threshold: 0.5\nmust_include_skills: [rust, async]";
        let cfg: MatchConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.must_include_skills, vec!["rust", "async"]);
    }

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

    #[test]
    fn linkedin_interactive_only_defaults_true() {
        let cfg = LinkedinSubmitConfig::default();
        assert!(
            cfg.interactive_only,
            "must default to assist-mode (true) so daemon never auto-clicks"
        );
    }

    #[test]
    fn linkedin_interactive_only_round_trips() {
        let yaml = "interactive_only: false";
        let cfg: LinkedinSubmitConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!cfg.interactive_only);
    }
}
