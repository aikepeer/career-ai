//! Provider request-body construction for the Rig gateway.

use serde_json::{json, Value};

use crate::error::Result;
use crate::types::{LlmRequest, LlmResponse};

use super::driver::{join_chat_completions_url, Http, RigLlm};
use super::error::reqwest_to_llm_err;
use super::response::{parse_anthropic_response, parse_openai_response};

impl RigLlm {
    /// Build the Anthropic `/v1/messages` request body. When `cache_profile`
    /// is true, the profile_block is sent as its own user message whose
    /// single content block carries `cache_control: {type: "ephemeral"}`,
    /// which is the documented form for prompt caching.
    pub(crate) fn anthropic_body(&self, req: &LlmRequest) -> Value {
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
            "model": self.effective_model(req),
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
    pub(crate) fn openai_body(&self, req: &LlmRequest) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        if !req.system.is_empty() {
            messages.push(json!({ "role": "system", "content": req.system }));
        }
        if !req.profile_block.is_empty() {
            messages.push(json!({ "role": "user", "content": req.profile_block }));
        }
        messages.push(json!({ "role": "user", "content": req.user }));
        json!({
            "model": self.effective_model(req),
            "messages": messages,
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
        })
    }

    pub(super) async fn send_once(&self, req: &LlmRequest) -> Result<LlmResponse> {
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
            Http::OpenAi(_client) => {
                let body = self.openai_body(req);
                let endpoint_url = match &self.api_base_url {
                    Some(base) => join_chat_completions_url(base),
                    None => "https://api.openai.com/v1/chat/completions".to_string(),
                };

                let http = reqwest::Client::new();
                let resp = http
                    .post(&endpoint_url)
                    .bearer_auth(&self.api_key)
                    .json(&body)
                    .send()
                    .await
                    .map_err(reqwest_to_llm_err)?;
                parse_openai_response(resp).await
            }
        }
    }
}
