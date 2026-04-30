//! Backend resolution and probing logic.
//!
//! Contains [`Backend::resolve`] (the entry-point that dispatches to
//! `resolve_auto`, `build_cli`, or `build_api`), [`Backend::probe`]
//! (the dry-run auto-detector used by `careerai llm probe`), and the
//! helper functions that pick models, strip provider prefixes, and
//! read API keys from env / keyring.

use std::sync::Arc;

use tracing::debug;
#[cfg(feature = "live-llm-api")]
use tracing::info;

use careerai_core::config::{BackendChoice, LlmConfig};

use crate::backend::{Backend, BackendError, BackendProbe};
use crate::cache::Cache;
#[cfg(feature = "live-llm-cli")]
use crate::claude_cli::{locate_claude_binary, ClaudeCliError, ClaudeCliLlm};
use crate::error::LlmError;
#[cfg(feature = "live-llm-api")]
use crate::rig::{Provider, RigLlm};

use super::probe::{last_ping_ms, probe_claude_auth};

impl Backend {
    /// Resolve a backend per the rules in this module's doc.
    ///
    /// `cfg.timeout_seconds` is forwarded to whichever driver wins.
    ///
    /// # Errors
    /// See [`BackendError`].
    pub async fn resolve(
        choice: BackendChoice,
        cfg: &LlmConfig,
        cache: Arc<Cache>,
    ) -> std::result::Result<Self, BackendError> {
        match choice {
            BackendChoice::Auto => resolve_auto(cfg, cache).await,
            BackendChoice::ClaudeCli => build_cli(cfg, cache).await,
            BackendChoice::Api => build_api(cfg, cache),
        }
    }

    pub async fn probe(cfg: &LlmConfig) -> BackendProbe {
        let mut probe = BackendProbe {
            chosen: BackendChoice::Auto,
            forced: None,
            claude_binary: None,
            claude_version: None,
            claude_ping_ms: None,
            claude_auth_ok: false,
            api_key_present: false,
            api_key_source: None,
        };

        #[cfg(feature = "live-llm-cli")]
        {
            use super::probe::read_claude_version;
            if let Ok(bin) = locate_claude_binary() {
                probe.claude_binary = Some(bin.clone());
                if let Some(v) = read_claude_version(&bin).await {
                    probe.claude_version = Some(v);
                }
                // Honor CAREERAI_SKIP_CLI_PROBE for parity with
                // `resolve_auto` and `build_cli`. The module doc claims
                // the env var is honored everywhere; this branch was
                // the lone exception. Skipping the live ping here gives
                // tests and offline scenarios a deterministic "binary
                // present" result without spawning claude.
                let auth_ok = std::env::var("CAREERAI_SKIP_CLI_PROBE").is_ok()
                    || probe_claude_auth(&bin).await;
                if auth_ok {
                    probe.claude_auth_ok = true;
                    probe.chosen = BackendChoice::ClaudeCli;
                    probe.claude_ping_ms = last_ping_ms();
                }
            }
        }

        // Always look for an API key, even when CLI wins, so `probe`
        // output can show the fallback status. Cache the result — each
        // `api_key_source()` call reads env + keyring, and on macOS the
        // keychain query can pop a confirmation dialog. One read per
        // probe.
        let api_source = api_key_source();
        probe.api_key_present = api_source.is_some();
        probe.api_key_source = api_source;
        if probe.api_key_present && probe.chosen == BackendChoice::Auto {
            #[cfg(feature = "live-llm-api")]
            {
                probe.chosen = BackendChoice::Api;
            }
        }

        // Track a forced backend choice from `cfg.backend` separately
        // from the auto-detected `chosen`. Previously `chosen` was
        // overwritten unconditionally with `cfg.backend`, which lied in
        // three cases:
        //   - forced `Api` but `live-llm-api` not compiled
        //   - forced `Api` but no API key
        //   - forced `ClaudeCli` but binary not present / not authed
        // Now `chosen` always reflects what would actually resolve, and
        // `forced` records what the operator asked for. The CLI prints
        // both so the operator can see when their override is unusable.
        if matches!(cfg.backend, BackendChoice::ClaudeCli | BackendChoice::Api) {
            probe.forced = Some(cfg.backend);
        }

        probe
    }
}

async fn resolve_auto(
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    #[cfg(feature = "live-llm-cli")]
    {
        if let Ok(bin) = locate_claude_binary() {
            let auth_ok =
                std::env::var("CAREERAI_SKIP_CLI_PROBE").is_ok() || probe_claude_auth(&bin).await;
            if auth_ok {
                debug!(
                    target = "careerai_llm::backend",
                    binary = %bin.display(),
                    "auto-resolved backend: claude-cli"
                );
                let model = pick_default_model(cfg);
                return Ok(Backend::ClaudeCli(ClaudeCliLlm::new(
                    bin,
                    model,
                    cache,
                    cfg.timeout_seconds.max(1),
                )));
            }
            debug!(
                target = "careerai_llm::backend",
                "claude binary present but ping failed; falling through"
            );
        }
    }

    #[cfg(feature = "live-llm-api")]
    {
        if api_key_source().is_some() {
            return build_api(cfg, cache);
        }
    }

    let _ = cache;
    let _ = cfg;
    Err(BackendError::NoneAvailable)
}

#[cfg(feature = "live-llm-cli")]
async fn build_cli(
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    let bin = locate_claude_binary().map_err(|e| match e {
        ClaudeCliError::NotInstalled => BackendError::CliMissing,
        other => BackendError::Llm(LlmError::from(other)),
    })?;

    if std::env::var("CAREERAI_SKIP_CLI_PROBE").is_err() && !probe_claude_auth(&bin).await {
        return Err(BackendError::CliNotAuthenticated);
    }

    let model = pick_default_model(cfg);
    Ok(Backend::ClaudeCli(ClaudeCliLlm::new(
        bin,
        model,
        cache,
        cfg.timeout_seconds.max(1),
    )))
}

#[cfg(not(feature = "live-llm-cli"))]
async fn build_cli(
    _cfg: &LlmConfig,
    _cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("claude-cli"))
}

#[cfg(feature = "live-llm-api")]
fn build_api(cfg: &LlmConfig, cache: Arc<Cache>) -> std::result::Result<Backend, BackendError> {
    let key = read_api_key().ok_or(BackendError::ApiKeyMissing)?;
    let model = pick_default_model(cfg);
    let llm = RigLlm::with_api_key(
        Provider::Anthropic,
        key,
        model,
        cache,
        cfg.timeout_seconds.max(1),
    )
    .map_err(BackendError::Llm)?;
    info!(
        target = "careerai_llm::backend",
        "auto-resolved backend: anthropic-api"
    );
    Ok(Backend::Api(llm))
}

#[cfg(not(feature = "live-llm-api"))]
fn build_api(_cfg: &LlmConfig, _cache: Arc<Cache>) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("api"))
}

/// Pick the default model for the resolved backend. Tailor model wins,
/// then parse_resume_model, then a hard-coded sensible default.
pub(crate) fn pick_default_model(cfg: &LlmConfig) -> String {
    let candidate = if !cfg.tailor_model.is_empty() {
        cfg.tailor_model.as_str()
    } else if !cfg.parse_resume_model.is_empty() {
        cfg.parse_resume_model.as_str()
    } else {
        "sonnet"
    };
    strip_provider_prefix(candidate).to_string()
}

/// Strip a leading `anthropic/` / `openai/` namespace if the layered
/// config used the rig-style multi-provider naming.
pub(crate) fn strip_provider_prefix(model: &str) -> &str {
    if let Some((_, rest)) = model.split_once('/') {
        rest
    } else {
        model
    }
}

/// Best-effort lookup for an Anthropic API key. Mirrors
/// `careerai-cli::anthropic_key_reachable` so behavior is consistent
/// across CLI and library callers.
#[cfg(feature = "live-llm-api")]
fn read_api_key() -> Option<String> {
    if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
        if !v.is_empty() {
            return Some(v);
        }
    }
    keyring::Entry::new("career-ai", "anthropic/api_key")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|v| !v.is_empty())
}

fn api_key_source() -> Option<&'static str> {
    if std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty())
    {
        return Some("env:ANTHROPIC_API_KEY");
    }
    let entry = keyring::Entry::new("career-ai", "anthropic/api_key").ok()?;
    let v = entry.get_password().ok()?;
    if v.is_empty() {
        None
    } else {
        Some("keyring:career-ai/anthropic/api_key")
    }
}
