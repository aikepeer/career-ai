//! CLI response-envelope parsing for Claude-compatible subprocess backends.

use crate::types::LlmResponse;

use super::error::{classify_error_payload, classify_failure_stderr, ClaudeCliError};
use super::response::{clamp_u64_u32, ClaudeCliResult};

pub(crate) fn parse_cli_output(
    stdout: &str,
    stderr: &str,
    success: bool,
) -> std::result::Result<LlmResponse, ClaudeCliError> {
    // The CLI emits its result JSON on stdout even on error
    // (`is_error: true`). Parse first; fall back to stderr-based
    // classification only if stdout isn't valid JSON.
    let parsed: ClaudeCliResult = match serde_json::from_str::<ClaudeCliResult>(stdout.trim()) {
        Ok(p) => p,
        Err(e) => {
            if !success {
                // No JSON at all: most likely binary missing or
                // crashed before printing.
                return Err(classify_failure_stderr(stderr));
            }
            return Err(ClaudeCliError::ParseJson(format!(
                "{e}; stdout={}",
                stdout.chars().take(256).collect::<String>()
            )));
        }
    };

    // agy's envelope carries `status`/`response`/`error` instead of
    // claude's `is_error`/`result`. Handle it before the claude
    // fields, which are `None` for agy.
    if let Some(status) = parsed.status.as_deref() {
        if status == "SUCCESS" {
            let text = parsed
                .response
                .clone()
                .ok_or_else(|| ClaudeCliError::ParseJson("agy missing `response` field".into()))?;
            let usage = parsed.usage.unwrap_or_default();
            return Ok(LlmResponse {
                text,
                prompt_tokens: clamp_u64_u32(usage.input_tokens),
                completion_tokens: clamp_u64_u32(usage.output_tokens),
                cache_hit: false,
                cached_prompt_tokens: clamp_u64_u32(usage.cache_read_input_tokens),
            });
        }
        return Err(classify_error_payload(&parsed));
    }

    // goose's envelope: a `messages` array where the last assistant
    // message carries the reply text, plus a `metadata` object with
    // token counts. Produced by `goose run --output-format json`.
    if !parsed.messages.is_empty() {
        let text = parsed
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .and_then(|m| {
                m.content
                    .iter()
                    .rev()
                    .find_map(|c| (!c.text.is_empty()).then(|| c.text.clone()))
            })
            .ok_or_else(|| {
                ClaudeCliError::ParseJson("goose: no assistant text found in messages".into())
            })?;
        let usage = parsed.goose_metadata.unwrap_or_default();
        if usage
            .status
            .as_deref()
            .is_some_and(|status| status != "completed")
        {
            return Err(ClaudeCliError::Transport(format!(
                "goose run status: {:?}",
                usage.status
            )));
        }
        return Ok(LlmResponse {
            text,
            prompt_tokens: usage.input_tokens.map_or(0, clamp_u64_u32),
            // Goose reports split input/output usage on normal completed
            // runs. Do not treat total_tokens as completion_tokens when
            // the split fields are absent; that would overstate output.
            completion_tokens: usage.output_tokens.map_or(0, clamp_u64_u32),
            cache_hit: false,
            cached_prompt_tokens: 0,
        });
    }

    if parsed.is_error.unwrap_or(false) {
        return Err(classify_error_payload(&parsed));
    }

    let text = parsed
        .result
        .clone()
        .ok_or_else(|| ClaudeCliError::ParseJson("missing `result` field".into()))?;

    let usage = parsed.usage.unwrap_or_default();

    Ok(LlmResponse {
        text,
        prompt_tokens: clamp_u64_u32(usage.input_tokens),
        completion_tokens: clamp_u64_u32(usage.output_tokens),
        cache_hit: false,
        cached_prompt_tokens: clamp_u64_u32(usage.cache_read_input_tokens),
    })
}
