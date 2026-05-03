//! Tool input/output schemas.
//!
//! Each tool exposed by the MCP server has its argument struct and result
//! struct here. The structs derive `schemars::JsonSchema` so `rmcp` can
//! advertise the JSON Schema to the LLM, plus `serde::{Deserialize,
//! Serialize}` for round-trip JSON-RPC framing.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

#[cfg(test)]
mod tests;
mod tools;

pub use tools::*;
