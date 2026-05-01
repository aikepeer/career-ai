//! LLM gateway configuration — backend selection + per-task model
//! / cache / retry knobs.

use serde::{Deserialize, Serialize};

/// Inference backend selector for the LLM gateway.
///
/// `Auto` lets `careerai-llm::Backend::resolve` decide:
/// 1. `claude` CLI on PATH and authed -> `ClaudeCli`
/// 2. `ANTHROPIC_API_KEY` reachable (env or keyring) -> `Api`
/// 3. Else error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendChoice {
    /// Detect at runtime; prefer `claude` CLI over API key.
    #[default]
    Auto,
    /// Force the `claude` CLI subprocess backend.
    ClaudeCli,
    /// Force the rig-core / Anthropic API backend (requires
    /// `ANTHROPIC_API_KEY`).
    Api,
}

impl BackendChoice {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ClaudeCli => "claude-cli",
            Self::Api => "api",
        }
    }
}

impl std::str::FromStr for BackendChoice {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "claude-cli" | "cli" | "claude" => Ok(Self::ClaudeCli),
            "api" | "anthropic-api" | "rig" => Ok(Self::Api),
            other => Err(format!(
                "unknown backend choice `{other}`; expected one of: auto, claude-cli, api"
            )),
        }
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
    /// Which inference backend to use: `auto`, `claude-cli`, or `api`.
    /// Defaults to `auto` — the `careerai-llm` resolver picks the
    /// `claude` CLI when the binary is reachable + authed, otherwise the
    /// rig-based API path. Surface a CLI override via
    /// `--llm-backend=<choice>`.
    #[serde(default)]
    pub backend: BackendChoice,
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
