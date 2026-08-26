//! JSON response types for `claude --print --output-format json`.

use serde::Deserialize;
use serde_json::Value;

/// Mirror of the relevant fields in the `claude --print --output-format json`
/// payload. Defensive: every field is `Option`, unknown fields tolerated.
///
/// Also parses `agy`'s envelope (`status`/`response`/`error`), the
/// Claude-Code-compatible Go CLI some users configure as `llm.backend`
/// (see `driver.rs`). The two shapes are mutually exclusive: claude
/// sends `is_error`/`result`, agy sends `status`/`response`/`error`.
///
/// Also parses `goose`'s envelope (`messages` array + `metadata`
/// object), produced by `goose run --output-format json`. Goose's
/// reply text lives in the last `assistant` message's `content[].text`.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AgentCliResult {
    #[serde(default)]
    pub(crate) is_error: Option<bool>,
    #[serde(default)]
    pub(crate) api_error_status: Option<u16>,
    #[serde(default)]
    pub(crate) result: Option<String>,
    /// agy: `"SUCCESS"` | `"ERROR"`.
    #[serde(default)]
    pub(crate) status: Option<String>,
    /// agy: the generated text on success.
    #[serde(default)]
    pub(crate) response: Option<String>,
    /// agy: the human-readable message on error.
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) usage: Option<AgentCliUsage>,
    /// goose: conversation messages (user + assistant turns). The
    /// last `assistant` message carries the reply text.
    #[serde(default)]
    pub(crate) messages: Vec<GooseMessage>,
    /// goose: run metadata (token counts + status). Renamed because
    /// claude/agy don't emit a top-level `metadata` key.
    #[serde(default, rename = "metadata")]
    pub(crate) goose_metadata: Option<GooseMetadata>,
    /// Catch-all so unknown keys (e.g. `modelUsage`, `terminal_reason`)
    /// don't break parsing.
    #[serde(flatten, default)]
    #[allow(dead_code)]
    pub(crate) extra: std::collections::BTreeMap<String, Value>,
}

pub(crate) type ClaudeCliResult = AgentCliResult;

/// goose `--output-format json` message entry. Only `role` and the
/// `text` content blocks are needed; other fields (id, created,
/// metadata) are ignored by serde's default unknown-field tolerance.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct GooseMessage {
    #[serde(default)]
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) content: Vec<GooseContent>,
}

/// goose message content block — only the `text` field is consumed.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct GooseContent {
    #[serde(default)]
    pub(crate) text: String,
}

/// goose run metadata: token usage + completion status.
#[derive(Debug, Clone, Default, Deserialize)]
#[allow(dead_code)]
pub(crate) struct GooseMetadata {
    #[serde(default)]
    pub(crate) input_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) output_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) total_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) status: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[allow(clippy::struct_field_names)] // mirrors the upstream JSON shape
pub(crate) struct AgentCliUsage {
    #[serde(default)]
    pub(crate) input_tokens: u64,
    #[serde(default)]
    pub(crate) output_tokens: u64,
    /// agy names this `cache_read_tokens`; accept both spellings.
    #[serde(default, alias = "cache_read_tokens")]
    pub(crate) cache_read_input_tokens: u64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    #[allow(dead_code)]
    pub(crate) cache_creation_input_tokens: u64,
}

#[allow(dead_code)]
pub(crate) type ClaudeCliUsage = AgentCliUsage;

pub(crate) fn clamp_u64_u32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}
