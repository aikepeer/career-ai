//! LLM gateway configuration — backend selection + per-task model
//! / cache / retry knobs.

use serde::{Deserialize, Serialize};

/// Inference backend selector for the LLM gateway.
///
/// `Auto` lets `careerai-llm::Backend::resolve` decide:
/// 1. `claude` CLI on PATH and authed -> `ClaudeCli`
/// 2. `ANTHROPIC_API_KEY` reachable (env or keyring) -> `Api`
/// 3. Else error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendChoice {
    /// Detect at runtime; prefer CLI over API key.
    #[default]
    Auto,
    /// Force the `claude` CLI subprocess backend.
    ClaudeCli,
    /// Force the API backend (Anthropic / OpenAI / DeepSeek / compatible endpoint).
    Api,
    /// Antigravity CLI agent (`agy`).
    Agy,
    /// OpenAI Codex / CLI.
    Codex,
    /// Pi Agent CLI (`pi`).
    Pi,
    /// Goose AI Agent CLI (`goose`).
    Goose,
    /// Grok / xAI CLI (`grok`).
    Grok,
    /// Aider AI Pair Programming CLI (`aider`).
    Aider,
    /// GitHub Copilot CLI (`copilot`).
    Copilot,
    /// Llama.cpp CLI (`llama-cpp` / `llama.cpp`).
    LlamaCpp,
    /// Custom CLI tool path or binary name (e.g. `~/.local/bin/agy`).
    CustomCli(String),
}

impl BackendChoice {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::ClaudeCli => "claude-cli",
            Self::Api => "api",
            Self::Agy => "agy",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Goose => "goose",
            Self::Grok => "grok",
            Self::Aider => "aider",
            Self::Copilot => "copilot",
            Self::LlamaCpp => "llama-cpp",
            Self::CustomCli(s) => s.as_str(),
        }
    }
}

impl std::str::FromStr for BackendChoice {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let trimmed = s.trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "claude-cli" | "cli" | "claude" => Ok(Self::ClaudeCli),
            "api" | "anthropic-api" | "openai-api" | "deepseek-api" | "rig" => Ok(Self::Api),
            "agy" => Ok(Self::Agy),
            "codex" => Ok(Self::Codex),
            "pi" => Ok(Self::Pi),
            "goose" => Ok(Self::Goose),
            "grok" => Ok(Self::Grok),
            "aider" => Ok(Self::Aider),
            "copilot" => Ok(Self::Copilot),
            "llama-cpp" | "llamacpp" | "llama.cpp" | "llama" => Ok(Self::LlamaCpp),
            _ if !trimmed.is_empty() => Ok(Self::CustomCli(trimmed.to_string())),
            _ => Err("backend choice cannot be empty".to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Generic provider identifier (e.g. "auto", "deepseek", "openai", "openrouter", "anthropic", "ollama")
    #[serde(default)]
    pub provider: String,
    /// Generic model identifier (e.g. "deepseek-chat", "gpt-4o-mini", "claude-3-5-sonnet")
    #[serde(default)]
    pub model: String,
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
    /// Tailor strategy: "llm" | "local" | "hybrid" (Phase 1 LLM reduction).
    #[serde(default = "default_tailor_strategy")]
    pub strategy: String,
    /// Bullet relevance threshold for local pruning [0.0, 1.0].
    #[serde(default = "default_drop_threshold")]
    pub drop_threshold: f32,
    /// Custom API Base URL for OpenAI/Anthropic/DeepSeek compatible endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    /// Custom API Key for API backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

fn default_tailor_strategy() -> String {
    "llm".to_string()
}
fn default_drop_threshold() -> f32 {
    0.05
}

fn default_cache_dir() -> String {
    "data/cache/llm".to_string()
}
fn default_max_retries() -> u32 {
    3
}
fn default_timeout_seconds() -> u64 {
    if let Ok(val) =
        std::env::var("LLM_TIMEOUT_SECONDS").or_else(|_| std::env::var("CAREERAI_LLM_TIMEOUT"))
    {
        if let Ok(parsed) = val.parse::<u64>() {
            return parsed;
        }
    }
    300
}
fn default_prompt_version() -> String {
    "tailor.v1".to_string()
}
fn default_anthropic_prompt_cache() -> bool {
    true
}
