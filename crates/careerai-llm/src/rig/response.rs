//! Anthropic and OpenAI response parsers.

use serde_json::Value;

use crate::error::{LlmError, Result};
use crate::types::LlmResponse;

use super::error::reqwest_to_llm_err;

pub(crate) async fn parse_anthropic_response(resp: reqwest::Response) -> Result<LlmResponse> {
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

pub(crate) async fn parse_openai_response(resp: reqwest::Response) -> Result<LlmResponse> {
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
