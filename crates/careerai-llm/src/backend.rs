//! Backend resolver — picks between the `claude` CLI subprocess driver
//! and the rig-core Anthropic API driver based on a [`BackendChoice`]
//! and what's reachable on the host.
//!
//! Resolution order when [`BackendChoice::Auto`]:
//!
//! 1. `which("claude")` succeeds AND a two-tier auth probe passes
//!    (skipped when `CAREERAI_SKIP_CLI_PROBE=1`) -> [`Backend::ClaudeCli`].
//!    The probe tries `claude auth status` first (cheap, <2s, zero
//!    token cost) and falls back to `claude --print "ping"` with a
//!    60s ceiling for older `claude` builds without `auth status`.
//! 2. `ANTHROPIC_API_KEY` is reachable (env or
//!    `keyring::Entry::new("career-ai", "anthropic/api_key")`) AND the
//!    `live-llm-api` feature is enabled -> [`Backend::Api`].
//! 3. Else [`BackendError::NoneAvailable`] with hints.
//!
//! When forced (`ClaudeCli` or `Api`): no fallback; mismatch is a hard
//! error.
//!
//! Implementation is split into submodules to keep this file small:
//!
//! * `auth` — the two-tier `claude auth status` / ping fallback probe.
//! * `key`  — Anthropic API key lookup (env + keyring).
//! * `build` — backend builders + `resolve_auto` + model-picking.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use careerai_core::config::{BackendChoice, LlmConfig};

use crate::cache::Cache;
use crate::error::{LlmError, Result as LlmResult};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

#[cfg(feature = "live-llm-cli")]
use crate::claude_cli::{locate_claude_binary, ClaudeCliLlm};
#[cfg(feature = "live-llm-api")]
use crate::rig::RigLlm;

pub(crate) mod auth;
pub(crate) mod build;
pub(crate) mod key;
#[cfg(test)]
mod tests;

/// Resolved backend, ready to issue LLM calls.
pub enum Backend {
    /// `claude` CLI subprocess.
    #[cfg(feature = "live-llm-cli")]
    ClaudeCli(ClaudeCliLlm),
    /// rig-core Anthropic API client.
    #[cfg(feature = "live-llm-api")]
    Api(RigLlm),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "live-llm-cli")]
            Self::ClaudeCli(_) => f.debug_tuple("Backend::ClaudeCli").finish(),
            #[cfg(feature = "live-llm-api")]
            Self::Api(_) => f.debug_tuple("Backend::Api").finish(),
            #[allow(unreachable_patterns)]
            _ => f.write_str("Backend::None"),
        }
    }
}

/// Errors specific to backend resolution.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error(
        "`claude` CLI not installed or not on PATH; install Claude Code or set CAREERAI_CLAUDE_BIN"
    )]
    CliMissing,

    #[error("`claude` CLI session is not authenticated; run `claude login`")]
    CliNotAuthenticated,

    #[error("ANTHROPIC_API_KEY not set in env or keyring")]
    ApiKeyMissing,

    #[error(
        "no LLM backend reachable: `claude` CLI not found AND no ANTHROPIC_API_KEY; \
         install Claude Code (https://claude.ai/download) OR export ANTHROPIC_API_KEY"
    )]
    NoneAvailable,

    #[error("backend `{0}` was forced but its feature is not compiled in")]
    FeatureDisabled(&'static str),

    #[error("llm: {0}")]
    Llm(#[from] LlmError),
}

/// Outcome of [`Backend::probe`] — what the auto-detector picked
/// without actually constructing the driver. Useful for `careerai llm
/// probe` and the plugin setup script.
///
/// `chosen` and `forced` are computed independently — do not infer one
/// from the other:
///
/// * `chosen` reflects strictly what `Backend::resolve(Auto, ...)` would
///   return on this host: claude binary on PATH and ping succeeded
///   (`ClaudeCli`), or an Anthropic API key reachable (`Api`), else
///   `Auto`. It does NOT consider `cfg.backend`.
/// * `forced` records the operator's override from `cfg.backend` (set
///   via `--llm-backend` or `cfg.llm.backend`). Independent of
///   `chosen`; may or may not match.
///
/// The two fields can legitimately disagree without implying the
/// override is broken. Whether the override actually works is answered
/// by calling `Backend::resolve(forced, ...)` — the CLI does this in
/// `probe_forced_resolve` and surfaces a non-zero exit only when that
/// resolve errors. Field comparison alone is not sufficient.
#[derive(Debug, Clone)]
pub struct BackendProbe {
    /// What `Backend::resolve(Auto, ...)` would hand back on this host
    /// (claude binary on PATH + ping ok, or API key reachable).
    /// Strictly auto-detection — does NOT consider `cfg.backend`.
    /// `Auto` means neither backend is reachable.
    pub chosen: BackendChoice,
    /// Operator override from `cfg.backend`, if not `Auto`. Independent
    /// of `chosen` — may legitimately differ when the operator forces a
    /// backend that auto-detection would not have picked.
    pub forced: Option<BackendChoice>,
    pub claude_binary: Option<PathBuf>,
    pub claude_version: Option<String>,
    pub claude_ping_ms: Option<u128>,
    pub claude_auth_ok: bool,
    /// True if an Anthropic API key is reachable (env or keyring).
    pub api_key_present: bool,
    pub api_key_source: Option<&'static str>,
}

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
            BackendChoice::Auto => build::resolve_auto(cfg, cache).await,
            BackendChoice::ClaudeCli => build::build_cli(cfg, cache).await,
            BackendChoice::Api => build::build_api(cfg, cache),
            other => build::build_named_cli(other.as_str(), cfg, cache),
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
            if let Ok(bin) = locate_claude_binary() {
                probe.claude_binary = Some(bin.clone());
                if let Some(v) = auth::read_claude_version(&bin).await {
                    probe.claude_version = Some(v);
                }
                // Honor CAREERAI_SKIP_CLI_PROBE for parity with
                // `resolve_auto` and `build_cli`. Skipping the live
                // ping here gives tests and offline scenarios a
                // deterministic "binary present" result without
                // spawning claude.
                let auth_ok = std::env::var("CAREERAI_SKIP_CLI_PROBE").is_ok()
                    || auth::probe_claude_auth(&bin).await;
                if auth_ok {
                    probe.claude_auth_ok = true;
                    probe.chosen = BackendChoice::ClaudeCli;
                    probe.claude_ping_ms = auth::last_ping_ms();
                }
            }
        }

        // Always look for an API key, even when CLI wins, so `probe`
        // output can show the fallback status. Also honor an explicit
        // key in `config/local.yaml` (`llm.api_key`).
        let api_source = key::api_key_source();
        probe.api_key_present =
            api_source.is_some() || cfg.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
        probe.api_key_source = api_source.or_else(|| {
            cfg.api_key
                .as_deref()
                .filter(|k| !k.trim().is_empty())
                .map(|_| "config:llm.api_key")
        });
        if probe.api_key_present && probe.chosen == BackendChoice::Auto {
            #[cfg(feature = "live-llm-api")]
            {
                probe.chosen = BackendChoice::Api;
            }
        }

        // Track a forced backend choice from `cfg.backend` separately
        // from the auto-detected `chosen`. `chosen` always reflects what
        // would actually resolve, and `forced` records what the operator
        // asked for. The CLI prints both so the operator can see when
        // their override is unusable.
        if cfg.backend != BackendChoice::Auto {
            probe.forced = Some(cfg.backend.clone());
        }

        probe
    }
}

#[async_trait]
impl Llm for Backend {
    fn name(&self) -> &'static str {
        match self {
            #[cfg(feature = "live-llm-cli")]
            Self::ClaudeCli(b) => b.name(),
            #[cfg(feature = "live-llm-api")]
            Self::Api(b) => b.name(),
            #[allow(unreachable_patterns)]
            _ => "none",
        }
    }

    async fn complete(&self, req: &LlmRequest) -> LlmResult<LlmResponse> {
        match self {
            #[cfg(feature = "live-llm-cli")]
            Self::ClaudeCli(b) => b.complete(req).await,
            #[cfg(feature = "live-llm-api")]
            Self::Api(b) => b.complete(req).await,
            #[allow(unreachable_patterns)]
            _ => Err(LlmError::Upstream("no backend feature compiled".into())),
        }
    }
}
