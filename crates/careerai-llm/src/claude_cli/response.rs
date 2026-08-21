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
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ClaudeCliResult {
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
    pub(crate) usage: Option<ClaudeCliUsage>,
    /// Catch-all so unknown keys (e.g. `modelUsage`, `terminal_reason`)
    /// don't break parsing.
    #[serde(flatten, default)]
    #[allow(dead_code)]
    pub(crate) extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[allow(clippy::struct_field_names)] // mirrors the upstream JSON shape
pub(crate) struct ClaudeCliUsage {
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

pub(crate) fn clamp_u64_u32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}
