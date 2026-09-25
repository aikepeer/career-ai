//! Generic agent and CLI subprocess backend for the [`Llm`] trait.
//!
//! Runs the user's locally-installed agent CLI binaries (`claude`, `agy`, `goose`,
//! `codex`, `pi`, `grok`, `aider`, `copilot`, `llama-cpp`, or custom executables)
//! in non-interactive batch mode and parses their structured result.

mod binary_locator;
pub(crate) mod driver;
mod error;
mod output;
mod response;
#[cfg(test)]
mod tests;
mod transport;

pub(crate) use binary_locator::{locate_claude_binary, locate_named_binary};
pub use driver::{AgentCliLlm, ClaudeCliLlm};
#[cfg(test)]
pub(crate) use error::classify_error_payload;
pub use error::{AgentCliError, ClaudeCliError};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use response::{AgentCliResult, ClaudeCliResult};
