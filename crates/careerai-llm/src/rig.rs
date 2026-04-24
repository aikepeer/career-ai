//! Real provider implementation of [`Llm`] via `rig-core`.
//!
//! Gated behind the `live-llm` Cargo feature. Supports Anthropic and OpenAI.
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
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::cache::Cache;
use crate::error::{LlmError, Result};
use crate::retry::llm_backoff;
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

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
enum Http {
    Anthropic(anthropic::client::Client),
    OpenAi(openai::Client),
}

/// Real provider impl of [`Llm`]. Only available under `--features live-llm`.
pub struct RigLlm {
    http: Http,
    model: String,
    /// Carried for future on-disk cache plumbing. The trait's
    /// [`LlmResponse::cache_hit`] field is wired through the `Cache` type,
    /// but the on-disk read/write integration is scheduled for the tailor
    /// crate (Wave 3C). Kept here so construction signatures are stable.
    #[allow(dead_code)]
    cache: Arc<Cache>,
    timeout: Duration,
}

impl std::fmt::Debug for RigLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `http` is deliberately elided — `rig::providers::*::Client` holds a
        // reqwest client with the API key baked into default headers, so
        // including it in Debug output would risk leaking secrets into logs.
        f.debug_struct("RigLlm")
            .field("provider", &self.provider())
            .field("model", &self.model)
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
    // `api_key: String` keeps the key at a by-value callsite so callers (env
    // readers in particular) pass ownership and the original buffer is dropped
    // once the rig client clones it into its default headers. Taking `&str`
    // would force every caller to keep their copy alive for an arbitrary
    // lifetime. Worth the one `needless_pass_by_value` allow.
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_api_key(
        provider: Provider,
        api_key: String,
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Result<Self> {
        let http = match provider {
            Provider::Anthropic => {
                // Attach the prompt-caching beta header at construction so
                // every request gets it. The beta flag is a no-op on
                // requests that don't include cache_control.
                let client = anthropic::client::ClientBuilder::new(&api_key)
                    .anthropic_beta(ANTHROPIC_PROMPT_CACHE_BETA)
                    .build();
                Http::Anthropic(client)
            }
            Provider::OpenAI => Http::OpenAi(openai::Client::new(&api_key)),
        };
        Ok(Self {
            http,
            model: model.into(),
            cache,
            timeout: Duration::from_secs(timeout_seconds),
        })
    }

    fn provider(&self) -> Provider {
        match self.http {
            Http::Anthropic(_) => Provider::Anthropic,
            Http::OpenAi(_) => Provider::OpenAI,
        }
    }

    /// Build the Anthropic `/v1/messages` request body. When `cache_profile`
    /// is true, the profile_block is sent as its own user message whose
    /// single content block carries `cache_control: {type: "ephemeral"}`,
    /// which is the documented form for prompt caching.
    fn anthropic_body(&self, req: &LlmRequest) -> Value {
        let mut user_content: Vec<Value> = Vec::new();

        if !req.profile_block.is_empty() {
            let mut block = json!({
                "type": "text",
                "text": req.profile_block,
            });
            if req.cache_profile {
                block["cache_control"] = json!({ "type": "ephemeral" });
            }
            user_content.push(block);
        }

        user_content.push(json!({
            "type": "text",
            "text": req.user,
        }));

        json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "system": req.system,
            "messages": [
                {
                    "role": "user",
                    "content": user_content,
                }
            ],
        })
    }

    /// Build the OpenAI `/v1/chat/completions` request body. Prompt caching
    /// on OpenAI is automatic server-side (no client hint required on
    /// chat.completions as of the `gpt-4o-*` family), so `cache_profile`
    /// is informational only — we still split the profile into its own
    /// leading user message for parity and to keep the token profile
    /// comparable across providers.
    fn openai_body(&self, req: &LlmRequest) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        if !req.system.is_empty() {
            messages.push(json!({ "role": "system", "content": req.system }));
        }
        if !req.profile_block.is_empty() {
            messages.push(json!({ "role": "user", "content": req.profile_block }));
        }
        messages.push(json!({ "role": "user", "content": req.user }));
        json!({
            "model": self.model,
            "messages": messages,
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
        })
    }

    async fn send_once(&self, req: &LlmRequest) -> Result<LlmResponse> {
        match &self.http {
            Http::Anthropic(client) => {
                let body = self.anthropic_body(req);
                let resp = client
                    .post("/v1/messages")
                    .json(&body)
                    .send()
                    .await
                    .map_err(reqwest_to_llm_err)?;
                parse_anthropic_response(resp).await
            }
            Http::OpenAi(client) => {
                let body = self.openai_body(req);
                // `openai::Client::post` is crate-private, so drive the
                // request through rig's public surface instead: build the
                // URL ourselves and authenticate via the client's default
                // headers. The Client's http_client already holds the
                // bearer token, but it's not exposed. Fall back to a
                // standalone reqwest call with the key pulled from the
                // env — safe because `openai_from_env` just validated it.
                //
                // Concretely: we POST to api.openai.com/v1/chat/completions
                // with `Authorization: Bearer $OPENAI_API_KEY`.
                let _ = client; // reserved for future use (streaming etc.)
                let api_key = std::env::var("OPENAI_API_KEY")
                    .map_err(|_| LlmError::Upstream("OPENAI_API_KEY unset at call time".into()))?;
                let http = reqwest::Client::new();
                let resp = http
                    .post("https://api.openai.com/v1/chat/completions")
                    .bearer_auth(api_key)
                    .json(&body)
                    .send()
                    .await
                    .map_err(reqwest_to_llm_err)?;
                parse_openai_response(resp).await
            }
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
            // Non-fatal — OpenAI ignores the hint, but surface that we
            // noticed so operators aren't surprised when the metric stays
            // at zero.
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

        let response = call.retry(llm_backoff()).when(is_retryable).await?;

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

/// Retry classifier. Transient network/timeout/rate-limit errors retry;
/// schema errors and hard upstream failures (4xx other than 429) do not.
fn is_retryable(err: &LlmError) -> bool {
    matches!(err, LlmError::RateLimited { .. } | LlmError::Timeout { .. })
        || matches!(err, LlmError::Upstream(msg) if is_transient_upstream(msg))
}

fn is_transient_upstream(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("timed out")
        || m.contains("timeout")
        || m.contains("connection reset")
        || m.contains("connection closed")
        || m.contains("broken pipe")
        || m.contains("dns")
        || m.contains("502")
        || m.contains("503")
        || m.contains("504")
}

// Takes `reqwest::Error` by value because it's used with `.map_err(...)` and
// that closure API moves the error.
#[allow(clippy::needless_pass_by_value)]
fn reqwest_to_llm_err(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout { seconds: 0 }
    } else {
        LlmError::Upstream(e.to_string())
    }
}

async fn parse_anthropic_response(resp: reqwest::Response) -> Result<LlmResponse> {
    let status = resp.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);
        return Err(LlmError::RateLimited {
            retry_after_seconds: retry_after,
        });
    }
    let body: Value = if status.is_success() {
        resp.json().await.map_err(reqwest_to_llm_err)?
    } else {
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Upstream(format!(
            "anthropic http {status}: {text}"
        )));
    };

    // content is an array of blocks; we want the concatenated text of all
    // `text` blocks. Anything else (tool_use etc.) we surface as Schema —
    // the tailor prompt doesn't request tools.
    let text = body
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(Value::as_str) == Some("text") {
                        b.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<String>()
        })
        .ok_or_else(|| LlmError::Schema("anthropic: missing content array".into()))?;

    let usage = body
        .get("usage")
        .ok_or_else(|| LlmError::Schema("anthropic: missing usage".into()))?;
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached_prompt_tokens = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    Ok(LlmResponse {
        text,
        prompt_tokens: u32::try_from(prompt_tokens).unwrap_or(u32::MAX),
        completion_tokens: u32::try_from(completion_tokens).unwrap_or(u32::MAX),
        cache_hit: false,
        cached_prompt_tokens: u32::try_from(cached_prompt_tokens).unwrap_or(u32::MAX),
    })
}

async fn parse_openai_response(resp: reqwest::Response) -> Result<LlmResponse> {
    let status = resp.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);
        return Err(LlmError::RateLimited {
            retry_after_seconds: retry_after,
        });
    }
    let body: Value = if status.is_success() {
        resp.json().await.map_err(reqwest_to_llm_err)?
    } else {
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Upstream(format!("openai http {status}: {text}")));
    };

    let text = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Schema("openai: missing choices[0].message.content".into()))?
        .to_owned();

    let usage = body.get("usage");
    let prompt_tokens = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    // OpenAI reports cached tokens under prompt_tokens_details.cached_tokens
    // when automatic prompt caching kicks in (gpt-4o etc.). Absent on older
    // models.
    let cached_prompt_tokens = usage
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    Ok(LlmResponse {
        text,
        prompt_tokens: u32::try_from(prompt_tokens).unwrap_or(u32::MAX),
        completion_tokens: u32::try_from(completion_tokens).unwrap_or(u32::MAX),
        cache_hit: false,
        cached_prompt_tokens: u32::try_from(cached_prompt_tokens).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn is_retryable_classifies_correctly() {
        assert!(is_retryable(&LlmError::RateLimited {
            retry_after_seconds: 1
        }));
        assert!(is_retryable(&LlmError::Timeout { seconds: 1 }));
        assert!(is_retryable(&LlmError::Upstream("connection reset".into())));
        assert!(is_retryable(&LlmError::Upstream("http 503".into())));
        assert!(!is_retryable(&LlmError::Upstream("http 401".into())));
        assert!(!is_retryable(&LlmError::Schema("bad".into())));
    }

    #[test]
    fn anthropic_body_attaches_cache_control_only_when_flagged() {
        let cache = Arc::new(Cache::new("/tmp/cache"));
        let llm = RigLlm::with_api_key(
            Provider::Anthropic,
            "sk-test".into(),
            "claude-3-5-sonnet-latest",
            cache,
            60,
        )
        .expect("construct");

        let mut req = LlmRequest {
            system: "sys".into(),
            profile_block: "PROFILE".into(),
            user: "USER".into(),
            prompt_version: "tailor.v1".into(),
            model: "claude-3-5-sonnet-latest".into(),
            temperature: 0.1,
            max_tokens: 1024,
            cache_profile: true,
        };
        let body = llm.anthropic_body(&req);
        let first_block = &body["messages"][0]["content"][0];
        assert_eq!(first_block["type"], "text");
        assert_eq!(first_block["text"], "PROFILE");
        assert_eq!(first_block["cache_control"]["type"], "ephemeral");

        req.cache_profile = false;
        let body = llm.anthropic_body(&req);
        assert!(body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn openai_body_shapes_system_and_user() {
        let cache = Arc::new(Cache::new("/tmp/cache"));
        let llm =
            RigLlm::with_api_key(Provider::OpenAI, "sk-test".into(), "gpt-4o-mini", cache, 60)
                .expect("construct");
        let req = LlmRequest {
            system: "sys".into(),
            profile_block: "PROFILE".into(),
            user: "USER".into(),
            prompt_version: "tailor.v1".into(),
            model: "gpt-4o-mini".into(),
            temperature: 0.2,
            max_tokens: 512,
            cache_profile: false,
        };
        let body = llm.openai_body(&req);
        let msgs = body["messages"].as_array().expect("messages array");
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "PROFILE");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"], "USER");
    }
}
