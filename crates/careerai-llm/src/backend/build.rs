//! Backend builders — assemble a concrete driver per
//! [`super::BackendChoice`].
//!
//! Resolution order when [`BackendChoice::Auto`]:
//!
//! 1. `which("claude")` succeeds AND a two-tier auth probe passes
//!    (skipped when `CAREERAI_SKIP_CLI_PROBE=1`) -> [`Backend::ClaudeCli`].
//! 2. `ANTHROPIC_API_KEY` is reachable (env or keyring) AND the
//!    `live-llm-api` feature is enabled -> [`Backend::Api`].
//! 3. Else [`BackendError::NoneAvailable`] with hints.

use std::sync::Arc;

use careerai_core::config::LlmConfig;
use tracing::debug;
#[cfg(feature = "live-llm-api")]
use tracing::info;

use crate::cache::Cache;

#[cfg(feature = "live-llm-cli")]
use crate::claude_cli::{locate_claude_binary, ClaudeCliError, ClaudeCliLlm};
#[cfg(feature = "live-llm-api")]
use crate::rig::{Provider, RigLlm};

#[cfg(feature = "live-llm-cli")]
use super::auth::probe_claude_auth;
#[cfg(feature = "live-llm-api")]
use super::key::{api_key_source, read_api_key};
use super::{Backend, BackendError};

pub(super) async fn resolve_auto(
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
pub(super) async fn build_cli(
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    let bin = locate_claude_binary().map_err(|e| match e {
        ClaudeCliError::NotInstalled => BackendError::CliMissing,
        other => BackendError::Llm(crate::error::LlmError::from(other)),
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
pub(super) async fn build_cli(
    _cfg: &LlmConfig,
    _cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("claude-cli"))
}

#[cfg(feature = "live-llm-api")]
pub(super) fn build_api(
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
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
pub(super) fn build_api(
    _cfg: &LlmConfig,
    _cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("api"))
}

/// Pick the default model for the resolved backend. Tailor model wins,
/// then parse_resume_model, then a hard-coded sensible default.
pub(super) fn pick_default_model(cfg: &LlmConfig) -> String {
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
pub(super) fn strip_provider_prefix(model: &str) -> &str {
    if let Some((_, rest)) = model.split_once('/') {
        rest
    } else {
        model
    }
}
