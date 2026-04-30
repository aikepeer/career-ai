//! Backend resolver — picks between the `claude` CLI subprocess driver
//! and the rig-core Anthropic API driver based on a [`BackendChoice`]
//! and what's reachable on the host.
//!
//! When forced (`ClaudeCli` or `Api`): no fallback; mismatch is a hard
//! error.
//!
//! ## Module layout
//!
//! * [`Backend`], [`BackendError`], [`BackendProbe`] — core types (this file).
//! * [`resolution`] — [`Backend::resolve`], [`Backend::probe`], auto-resolver,
//!   builder helpers, model picker, API-key reader.
//! * [`probe`] — Claude CLI auth probe (two-tier: `auth status` → ping fallback),
//!   version reader, latency tracking.

use std::path::PathBuf;

use async_trait::async_trait;
use careerai_core::config::BackendChoice;

#[cfg(feature = "live-llm-cli")]
use crate::claude_cli::ClaudeCliLlm;
use crate::error::{LlmError, Result as LlmResult};
#[cfg(feature = "live-llm-api")]
use crate::rig::RigLlm;
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

mod probe;
pub(crate) mod resolution;
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
/// override is broken. For example, with `forced = Some(Api)` and
/// `chosen = ClaudeCli`, auto-detection would pick the CLI, but the
/// operator forced API; that path can still resolve successfully when
/// `api_key_present = true`. Whether the override actually works is
/// answered by calling `Backend::resolve(forced, ...)` — the CLI does
/// this in `probe_forced_resolve` and surfaces a non-zero exit only
/// when that resolve errors. Field comparison alone is not sufficient.
#[derive(Debug, Clone)]
pub struct BackendProbe {
    /// What `Backend::resolve(Auto, ...)` would hand back on this host
    /// (claude binary on PATH + ping ok, or API key reachable).
    /// Strictly auto-detection — does NOT consider `cfg.backend`.
    /// `Auto` means neither backend is reachable.
    pub chosen: BackendChoice,
    /// Operator override from `cfg.backend`, if not `Auto`. Independent
    /// of `chosen` — may legitimately differ when the operator forces a
    /// backend that auto-detection would not have picked. Whether the
    /// override actually resolves is determined by calling
    /// `Backend::resolve(forced, ...)`, not by comparing fields.
    pub forced: Option<BackendChoice>,
    pub claude_binary: Option<PathBuf>,
    pub claude_version: Option<String>,
    pub claude_ping_ms: Option<u128>,
    pub claude_auth_ok: bool,
    /// True if an Anthropic API key is reachable (env or keyring). Set
    /// in tandem with `api_key_source` from a single cached read of
    /// `api_key_source()`.
    pub api_key_present: bool,
    pub api_key_source: Option<&'static str>,
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
