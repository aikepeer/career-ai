//! `careerai-mcp` — MCP (Model Context Protocol) server exposing the
//! career-ai pipeline as tool calls + resources.
//!
//! The crate's binary (`careerai-mcp`) speaks JSON-RPC over stdio and
//! is intended to be registered with Claude Code (or any MCP client)
//! so the LLM can drive discovery, tailoring, rendering, and apply
//! through structured tool calls. All business logic lives in the
//! existing pipeline crates; this crate is a thin adapter.
//!
//! Safety gates:
//!
//! - `careerai_apply` defaults to `dry_run = true`. Real submission
//!   requires both `dry_run = false` AND
//!   `confirm = "I_UNDERSTAND_TOS_RISK"`.
//! - Per-source `submit_enabled` config gates always apply.
//! - Errors are typed via [`error::McpServerError`]; the server never
//!   panics on bad input.

pub mod digest;
pub mod error;
pub mod schema;
pub mod server;

pub use error::McpServerError;
pub use server::CareerAiServer;
