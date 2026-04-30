//! MCP (Model Context Protocol) job-search server source config.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// One MCP-server discovery source. Spawns the configured stdio
/// process, performs an MCP `initialize` + `tools/list` handshake, and
/// invokes the first tool whose name matches one of the well-known
/// job-search aliases (`search_jobs`, `discover_jobs`, `find_jobs`,
/// `list_jobs`, `jobs.search`).
///
/// Defaults to `enabled: false` because community MCP servers vary in
/// trustworthiness and ToS exposure. The user opts in per-source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpSourceConfig {
    /// Stable identifier used as `Source::name()` and as the rate-limit
    /// bucket key. Example: `"linkedin-jobs-mcp"`. Must be unique
    /// within the `sources.mcp` list.
    pub name: String,
    /// Off by default. The daemon skips disabled sources entirely.
    #[serde(default)]
    pub enabled: bool,
    /// Discovery-only flag. The submit pipeline never calls these
    /// servers; this mirrors `submit.per_source.<name>.enabled` to
    /// avoid downstream confusion.
    #[serde(default)]
    pub submit_enabled: bool,
    /// Optional per-source cron expression. When set, the scheduler
    /// registers a job for this MCP source on this cadence and the
    /// override wins over any matching entry in `scheduler.cadence`
    /// keyed by `name`. When unset, the scheduler falls back to
    /// `scheduler.cadence.<name>` (and skips the source if that is also
    /// unset). Wired in `careerai-scheduler::Scheduler::from_config`
    /// via `effective_cadence`.
    #[serde(default)]
    pub cron: Option<String>,
    /// Soft cap on `tools/call` invocations per minute against this
    /// server. `0` disables the gate. Read-side caps live here (not in
    /// `rates.*`) because `rates.*` is sized for write-side submitters.
    #[serde(default)]
    pub rate_per_minute: u32,
    /// MCP transport + tool-input shape.
    pub mcp: McpTransportConfig,
}

/// stdio transport details + the job-search tool input shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpTransportConfig {
    /// Executable to spawn (e.g. `"uvx"`, `"npx"`, `"docker"`).
    pub command: String,
    /// Arguments passed to the executable.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables. Values support shell-style
    /// `${VAR}` expansion against the parent's environment.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Search query parameters forwarded to the tool call.
    #[serde(default)]
    pub query: McpQueryConfig,
}

/// Permissive search-query shape. Most community job-search MCP tools
/// accept some subset of `{ keywords, location, limit }`; we send all
/// three and let the server ignore extras.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpQueryConfig {
    #[serde(default)]
    pub keywords: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}
