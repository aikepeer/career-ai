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
use crate::agent_cli::{locate_claude_binary, AgentCliError, AgentCliLlm};
#[cfg(feature = "live-llm-api")]
use crate::rig::{Provider, RigLlm};

#[cfg(feature = "live-llm-cli")]
use super::auth::probe_claude_auth;
#[cfg(feature = "live-llm-api")]
use super::key::{api_key_source, detect_provider, read_api_key};
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
                let model = pick_default_model(cfg, Provider::Anthropic);
                return Ok(Backend::ClaudeCli(AgentCliLlm::new_with_options(
                    bin,
                    model,
                    cfg.provider.clone(),
                    cfg.effort.clone(),
                    cache,
                    cfg.timeout_seconds.max(1),
                    cfg.max_retries,
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
        // A key explicitly set in config (`llm.api_key`, e.g. written by the
        // dashboard) is a first-class key source for auto mode, mirroring
        // `build_api`. Previously only env/keyring were consulted here, so a
        // config-file key silently produced `NoneAvailable`.
        let config_key_present = cfg.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
        if config_key_present || api_key_source().is_some() {
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
        AgentCliError::NotInstalled => BackendError::CliMissing,
        other => BackendError::Llm(crate::error::LlmError::from(other)),
    })?;

    if std::env::var("CAREERAI_SKIP_CLI_PROBE").is_err() && !probe_claude_auth(&bin).await {
        return Err(BackendError::CliNotAuthenticated);
    }

    let model = pick_default_model(cfg, Provider::Anthropic);
    Ok(Backend::ClaudeCli(AgentCliLlm::new_with_options(
        bin,
        model,
        cfg.provider.clone(),
        cfg.effort.clone(),
        cache,
        cfg.timeout_seconds.max(1),
        cfg.max_retries,
    )))
}

#[cfg(feature = "live-llm-cli")]
pub(super) fn build_named_cli(
    name: &str,
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    let bin = crate::agent_cli::locate_named_binary(name).map_err(|e| match e {
        AgentCliError::NotInstalled => BackendError::CliMissing,
        other => BackendError::Llm(crate::error::LlmError::from(other)),
    })?;

    let model = pick_default_model(cfg, Provider::Anthropic);
    Ok(Backend::ClaudeCli(AgentCliLlm::new_with_options(
        bin,
        model,
        cfg.provider.clone(),
        cfg.effort.clone(),
        cache,
        cfg.timeout_seconds.max(1),
        cfg.max_retries,
    )))
}

#[cfg(not(feature = "live-llm-cli"))]
pub(super) fn build_named_cli(
    _name: &str,
    _cfg: &LlmConfig,
    _cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("claude-cli"))
}

#[cfg(not(feature = "live-llm-cli"))]
// The caller (`Backend::resolve`) awaits this uniformly; the feature-gated
// real implementation is async, so this stub must keep the async signature.
#[allow(clippy::unused_async)]
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
    // Prefer the explicit config-file key, falling back to env/keyring.
    // Previously only env/keyring were consulted, so a key in
    // `config/local.yaml` (llm.api_key) was silently ignored.
    let key = cfg
        .api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or_else(read_api_key)
        .ok_or(BackendError::ApiKeyMissing)?;
    let provider = provider_from_config(&cfg.provider).unwrap_or_else(detect_provider);
    let model = pick_default_model(cfg, provider);
    let llm = RigLlm::with_api_key_and_base_url_with_retries(
        provider,
        key,
        model,
        cfg.api_base_url.clone(),
        cache,
        cfg.timeout_seconds.max(1),
        cfg.max_retries,
    )
    .map_err(BackendError::Llm)?;
    info!(
        target = "careerai_llm::backend",
        ?provider,
        "resolved API backend"
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
pub(super) fn pick_default_model(cfg: &LlmConfig, _provider: Provider) -> String {
    if let Ok(m) = std::env::var("LLM_MODEL")
        .or_else(|_| std::env::var("CAREERAI_LLM_MODEL"))
        .or_else(|_| std::env::var("MODEL"))
    {
        if !m.trim().is_empty() {
            return strip_provider_prefix(m.trim()).to_string();
        }
    }
    let candidate = if !cfg.model.is_empty() {
        cfg.model.as_str()
    } else if !cfg.tailor_model.is_empty() {
        cfg.tailor_model.as_str()
    } else if !cfg.parse_resume_model.is_empty() {
        cfg.parse_resume_model.as_str()
    } else {
        ""
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

/// Map a `llm.provider` config string to a concrete [`Provider`]. Returns
/// `None` for `auto`/empty so the caller can fall back to env-based
/// detection.
#[cfg(feature = "live-llm-api")]
fn provider_from_config(s: &str) -> Option<Provider> {
    match s.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => Some(Provider::Anthropic),
        "openai" | "deepseek" | "openrouter" | "groq" | "grok" | "xai" | "ollama"
        | "openai-compatible" => Some(Provider::OpenAI),
        _ => None,
    }
}

#[cfg(all(test, feature = "live-llm-api"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn provider_from_config_maps_common_names() {
        assert_eq!(provider_from_config("deepseek"), Some(Provider::OpenAI));
        assert_eq!(provider_from_config("openai"), Some(Provider::OpenAI));
        assert_eq!(provider_from_config("openrouter"), Some(Provider::OpenAI));
        assert_eq!(provider_from_config("anthropic"), Some(Provider::Anthropic));
        assert_eq!(provider_from_config("auto"), None);
        assert_eq!(provider_from_config(""), None);
        assert_eq!(provider_from_config("  DeepSeek  "), Some(Provider::OpenAI));
    }
}
