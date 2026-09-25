//! Real provider impl of [`Llm`]: [`RigLlm`], [`Provider`] and [`Http`]
//! enums, body builders, and the send-once transport.
//!
//! Gated behind the `live-llm-api` Cargo feature. Supports Anthropic and
//! OpenAI.
//!
//! # Why we talk to the provider HTTP API directly
//!
//! `rig-core` 0.10 exposes providers as concrete `Client` structs under
//! `rig::providers::{anthropic,openai}` that assemble authenticated
//! `reqwest::Client`s. Its high-level `CompletionModel` trait wraps the
//! request body in a provider-specific envelope that does **not** surface
//! Anthropic's `cache_control` on individual content blocks — the field
//! exists as a type (`anthropic::completion::CacheControl::Ephemeral`) but
//! the completion path does not serialize it. To wire prompt caching we
//! build the `/v1/messages` body ourselves and `POST` via the rig client's
//! public `post()` helper, which keeps the rig-managed headers (`x-api-key`,
//! `anthropic-version`, `anthropic-beta`) intact. OpenAI goes through the
//! same low-level shape for parity. This stays inside rig's public API; no
//! unsafe assumptions about internals.
//!
//! Prompt caching only activates when `req.cache_profile` is true and the
//! provider is Anthropic, and only on the `profile_block` segment. The
//! `anthropic-beta: prompt-caching-2024-07-31` header is attached at client
//! construction.

use std::sync::Arc;
use std::time::Duration;

use backon::Retryable;
use rig::providers::{anthropic, openai};
use tracing::{info, warn};

use crate::cache::Cache;
use crate::error::{LlmError, Result};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

use super::error::is_retryable;

/// Anthropic prompt-caching beta header value, matches the value used by
/// rig's own examples and Anthropic's public docs.
const ANTHROPIC_PROMPT_CACHE_BETA: &str = "prompt-caching-2024-07-31";

/// Which upstream we're talking to. Selected at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAI,
}

/// Shared HTTP handle for the two providers. `rig::providers::*::Client`
/// owns the reqwest client with the right default headers.
#[derive(Clone)]
pub(super) enum Http {
    Anthropic(anthropic::client::Client),
    OpenAi(openai::Client),
}

/// Real provider impl of [`Llm`]. Only available under `--features live-llm-api`.
pub struct RigLlm {
    pub(super) http: Http,
    pub(super) api_key: String,
    pub(super) api_base_url: Option<String>,
    pub(super) model: String,
    /// Carried for future on-disk cache plumbing. The trait's
    /// [`LlmResponse::cache_hit`] field is wired through the `Cache` type,
    /// but the on-disk read/write integration is scheduled for the tailor
    /// crate (Wave 3C). Kept here so construction signatures are stable.
    #[allow(dead_code)]
    pub(super) cache: Arc<Cache>,
    pub(super) timeout: Duration,
    pub(super) max_retries: u32,
}

impl std::fmt::Debug for RigLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `http` is deliberately elided — `rig::providers::*::Client` holds a
        // reqwest client with the API key baked into default headers, so
        // including it in Debug output would risk leaking secrets into logs.
        f.debug_struct("RigLlm")
            .field("provider", &self.provider())
            .field("model", &self.model)
            .field("base_url", &self.api_base_url)
            .field("cache", &"<cache>")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl RigLlm {
    /// Construct an Anthropic client reading `ANTHROPIC_API_KEY` from env.
    /// Returns [`LlmError::Upstream`] with an actionable message if the
    /// variable is missing.
    ///
    /// # Errors
    ///
    /// Errors when the `ANTHROPIC_API_KEY` env var is unset.
    pub fn anthropic_from_env(
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            LlmError::Upstream(
                "ANTHROPIC_API_KEY not set; export it or drop cache_profile=false".into(),
            )
        })?;
        Self::with_api_key(Provider::Anthropic, key, model, cache, timeout_seconds)
    }

    /// Construct an OpenAI client reading `OPENAI_API_KEY` from env.
    ///
    /// # Errors
    ///
    /// Errors when the `OPENAI_API_KEY` env var is unset.
    pub fn openai_from_env(
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Result<Self> {
        let key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| LlmError::Upstream("OPENAI_API_KEY not set".into()))?;
        Self::with_api_key(Provider::OpenAI, key, model, cache, timeout_seconds)
    }

    /// Construct with an explicit API key. Used by tests that point at a
    /// mock HTTP server.
    ///
    /// # Errors
    ///
    /// Currently infallible but returns `Result` so future validation (e.g.
    /// empty-key rejection) can be added without a signature change.
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_api_key(
        provider: Provider,
        api_key: String,
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Result<Self> {
        Self::with_api_key_and_base_url(provider, api_key, model, None, cache, timeout_seconds)
    }

    /// Construct an API driver with an explicit retry budget.
    pub fn with_api_key_and_base_url_with_retries(
        provider: Provider,
        api_key: String,
        model: impl Into<String>,
        base_url_override: Option<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
        max_retries: u32,
    ) -> Result<Self> {
        Ok(Self::with_api_key_and_base_url_internal(
            provider,
            api_key,
            model,
            base_url_override,
            cache,
            timeout_seconds,
            max_retries,
        ))
    }

    /// Like [`Self::with_api_key`] but accepts an explicit base URL override
    /// (e.g. `config.local.yaml`'s `llm.api_base_url`). When `None`, the base
    /// URL is resolved from the standard env vars.
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_api_key_and_base_url(
        provider: Provider,
        api_key: String,
        model: impl Into<String>,
        base_url_override: Option<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Result<Self> {
        Ok(Self::with_api_key_and_base_url_internal(
            provider,
            api_key,
            model,
            base_url_override,
            cache,
            timeout_seconds,
            3,
        ))
    }

    fn with_api_key_and_base_url_internal(
        provider: Provider,
        api_key: String,
        model: impl Into<String>,
        base_url_override: Option<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
        max_retries: u32,
    ) -> Self {
        let base_url = base_url_override
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("LLM_API_BASE_URL")
                    .or_else(|_| std::env::var("LLM_BASE_URL"))
                    .or_else(|_| std::env::var("CAREERAI_LLM_API_BASE_URL"))
                    .or_else(|_| std::env::var("DEEPSEEK_API_BASE_URL"))
                    .or_else(|_| std::env::var("OPENAI_API_BASE_URL"))
                    .or_else(|_| std::env::var("OPENROUTER_API_BASE_URL"))
                    .or_else(|_| std::env::var("OLLAMA_API_BASE_URL"))
                    .or_else(|_| std::env::var("ANTHROPIC_BASE_URL"))
                    .ok()
            });

        let http = match provider {
            Provider::Anthropic => {
                let mut builder = anthropic::client::ClientBuilder::new(&api_key)
                    .anthropic_beta(ANTHROPIC_PROMPT_CACHE_BETA);
                if let Some(ref url) = base_url {
                    builder = builder.base_url(url);
                }
                Http::Anthropic(builder.build())
            }
            Provider::OpenAI => {
                let client = match base_url {
                    Some(ref url) => openai::Client::from_url(&api_key, url),
                    None => openai::Client::new(&api_key),
                };
                Http::OpenAi(client)
            }
        };
        Self {
            http,
            api_key,
            api_base_url: base_url,
            model: model.into(),
            cache,
            timeout: Duration::from_secs(crate::normalized_timeout_seconds(timeout_seconds)),
            max_retries,
        }
    }

    fn provider(&self) -> Provider {
        match self.http {
            Http::Anthropic(_) => Provider::Anthropic,
            Http::OpenAi(_) => Provider::OpenAI,
        }
    }

    /// The model to actually send: the per-request model (e.g.
    /// `tailor_model` / `cover_letter_model`) wins, falling back to the
    /// backend default resolved at construction time.
    pub(crate) fn effective_model<'a>(&'a self, req: &'a LlmRequest) -> &'a str {
        if req.model.trim().is_empty() {
            &self.model
        } else {
            &req.model
        }
    }
}

#[async_trait::async_trait]
impl Llm for RigLlm {
    fn name(&self) -> &'static str {
        "rig"
    }

    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        if req.cache_profile && self.provider() != Provider::Anthropic {
            warn!(
                target: "llm",
                provider = ?self.provider(),
                "cache_profile=true requested on non-Anthropic provider; ignored"
            );
        }

        let timeout = self.timeout;
        let call = || async {
            tokio::time::timeout(timeout, self.send_once(req))
                .await
                .unwrap_or(Err(LlmError::Timeout {
                    seconds: timeout.as_secs(),
                }))
        };

        let response = call
            .retry(crate::retry::llm_backoff_with_retries(self.max_retries))
            .when(is_retryable)
            .await?;

        info!(
            target: "llm",
            model = %self.model,
            prompt_tokens = response.prompt_tokens,
            completion_tokens = response.completion_tokens,
            cached_prompt_tokens = response.cached_prompt_tokens,
            "llm.complete"
        );

        Ok(response)
    }
}

pub(crate) fn join_chat_completions_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

#[cfg(test)]
mod url_tests {
    use super::join_chat_completions_url;

    #[test]
    fn test_join_chat_completions_url() {
        assert_eq!(
            join_chat_completions_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            join_chat_completions_url("https://api.deepseek.com/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            join_chat_completions_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            join_chat_completions_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            join_chat_completions_url("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }
}
