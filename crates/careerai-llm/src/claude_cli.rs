//! `claude` CLI subprocess backend for the [`Llm`] trait.
//!
//! Runs the user's locally-installed `claude` binary (Anthropic's Claude
//! Code CLI) in `--print --output-format json` mode and parses its result.
//! When the user is logged in via `claude login` (Max/Pro subscription or
//! API key), every call here bills against that session — no
//! `ANTHROPIC_API_KEY` is required in the environment.
//!
//! ## Module layout
//!
//! * [`driver`] — [`ClaudeCliLlm`] struct, `Llm` trait impl.
//! * [`error`] — [`ClaudeCliError`] types, `From` mapping, error classifiers.
//! * [`response`] — `ClaudeCliResult`, `ClaudeCliUsage` JSON payload types.
//! * [`binary_locator`] — locate + validate the `claude` binary on the host.
//!
//! # Caching
//!
//! Anthropic's prompt-cache `cache_control` blocks are an API-only feature
//! and are not exposed by the CLI surface; `LlmRequest::cache_profile=true`
//! is a no-op here (logged once via `tracing::debug!`). The on-disk
//! response cache lives in the consumer wrapper (e.g.
//! `careerai-tailor::tailor_for_listing`), keyed by
//! [`compose_key`](crate::hashing::compose_key); this driver does not
//! add a second layer.

mod binary_locator;
pub(crate) mod driver;
mod error;
mod response;
#[cfg(test)]
mod tests;

pub(crate) use binary_locator::locate_claude_binary;
pub use driver::ClaudeCliLlm;
#[cfg(test)]
pub(crate) use error::classify_error_payload;
pub use error::ClaudeCliError;
#[cfg(test)]
pub(crate) use response::ClaudeCliResult;
