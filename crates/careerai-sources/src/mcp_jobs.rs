//! MCP-server discovery adapter.
//!
//! Reads jobs from a remote stdio Model-Context-Protocol server. The
//! server is spawned per `discover()` call; we negotiate `initialize`,
//! list its tools, pick the first job-search-shaped tool by name, and
//! invoke it with the configured query. The response is decoded into
//! `RawListing`s using a permissive field-mapping that tolerates the
//! schema drift between community MCP servers (e.g. RapidAPI's
//! linkedin-jobs vs. mcp-linkedin vs. generic ATS proxies).
//!
//! The adapter NEVER runs the local careerai-mcp server — that one is
//! consumed by Claude. This adapter consumes OTHER people's MCP
//! servers as discovery feeds for the daemon.

mod discover;
mod parser;
mod source;
#[cfg(test)]
mod tests;

pub use discover::{probe, ProbeReport};
pub use source::McpJobsSource;
