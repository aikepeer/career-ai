//! Subprocess-backed `Llm` impl that shells out to the user's
//! `claude` CLI (`--print --output-format json`). Also supports
//! `agy`, a Claude-Code-compatible Go CLI configured as
//! `llm.backend: agy` (see `send_once` for its flag + envelope
//! differences).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tracing::debug;

use crate::cache::Cache;

use super::binary_locator::locate_claude_binary;
use super::error::ClaudeCliError;

/// Subprocess-backed `Llm` impl that shells out to CLI tools and agents
/// (`claude`, `agy`, `goose`, `codex`, `pi`, `grok`, etc.).
pub struct AgentCliLlm {
    pub(crate) binary: PathBuf,
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) effort: String,
    pub(crate) cache: Arc<Cache>,
    pub(crate) timeout: Duration,
    pub(crate) max_retries: u32,
}

/// Backwards-compatible type alias for `AgentCliLlm`.
pub type ClaudeCliLlm = AgentCliLlm;

impl std::fmt::Debug for AgentCliLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCliLlm")
            .field("binary", &self.binary)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl AgentCliLlm {
    /// Construct a driver pointing at `binary` (typically
    /// `which("claude")` resolved). `model` may be either an alias
    /// (`sonnet`, `haiku`, `opus`) or a full model id
    /// (`claude-sonnet-4-6`); the CLI accepts both via `--model`.
    pub fn new(
        binary: impl Into<PathBuf>,
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Self {
        Self::new_with_options(
            binary,
            model,
            String::new(),
            String::new(),
            cache,
            timeout_seconds,
            3,
        )
    }

    /// Construct a driver with provider, effort and retry settings from application configuration.
    pub fn new_with_options(
        binary: impl Into<PathBuf>,
        model: impl Into<String>,
        provider: impl Into<String>,
        effort: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
        max_retries: u32,
    ) -> Self {
        Self {
            binary: binary.into(),
            model: model.into(),
            provider: provider.into(),
            effort: effort.into(),
            cache,
            timeout: Duration::from_secs(crate::normalized_timeout_seconds(timeout_seconds)),
            max_retries,
        }
    }

    /// Convenience constructor: probe `PATH` for `claude` (or honor the
    /// `CAREERAI_CLAUDE_BIN` env override used by tests).
    ///
    /// # Errors
    /// Returns [`ClaudeCliError::NotInstalled`] when the binary cannot be
    /// resolved, or [`ClaudeCliError::BinaryUnusable`] when the resolved
    /// path is not a regular executable file.
    pub fn discover(
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> std::result::Result<Self, ClaudeCliError> {
        let binary = locate_claude_binary()?;
        Ok(Self::new(binary, model, cache, timeout_seconds))
    }
}

/// One-shot debug log noting that prompt caching isn't available on the
/// CLI backend. Repeated `cache_profile=true` requests across the same
/// process don't spam the log.
pub(crate) fn log_no_prompt_cache_once() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        debug!(
            target = "careerai_llm::claude_cli",
            "anthropic prompt cache unavailable on claude-cli backend (cache_profile flag is silently ignored)"
        );
    });
}
