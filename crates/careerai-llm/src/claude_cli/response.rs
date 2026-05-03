//! JSON response types for `claude --print --output-format json`.

use serde::Deserialize;
use serde_json::Value;

/// Mirror of the relevant fields in the `claude --print --output-format json`
/// payload. Defensive: every field is `Option`, unknown fields tolerated.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ClaudeCliResult {
    #[serde(default)]
    pub(crate) is_error: Option<bool>,
    #[serde(default)]
    pub(crate) api_error_status: Option<u16>,
    #[serde(default)]
    pub(crate) result: Option<String>,
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
    #[serde(default)]
    pub(crate) cache_read_input_tokens: u64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    #[allow(dead_code)]
    pub(crate) cache_creation_input_tokens: u64,
}

pub(crate) fn clamp_u64_u32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}
