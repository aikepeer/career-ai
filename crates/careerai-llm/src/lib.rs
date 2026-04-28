//! LLM gateway.
//!
//! Single call site for all LLM traffic. Provider-agnostic `Llm` trait with
//! `MockLlm` as the default deterministic impl. Two real backends behind
//! cargo features:
//!
//! - `live-llm-cli` (default): subprocess driver for the `claude` CLI;
//!   reuses the user's logged-in Claude Code session (Max/Pro or API key).
//!   No `ANTHROPIC_API_KEY` required.
//! - `live-llm-api`: rig-core Anthropic API client; requires
//!   `ANTHROPIC_API_KEY`.
//!
//! [`Backend::resolve`] picks between them at runtime.
//!
//! On-disk response cache keyed by sha256 of (prompt_version,
//! profile_hash, jd_hash, model) is shared across both backends.

#![forbid(unsafe_code)]

#[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
pub mod backend;
pub mod cache;
#[cfg(feature = "live-llm-cli")]
pub mod claude_cli;
pub mod error;
pub mod hashing;
pub mod mock;
pub mod retry;
#[cfg(feature = "live-llm-api")]
pub mod rig;
pub mod trait_def;
pub mod types;

#[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
pub use crate::backend::{Backend, BackendError, BackendProbe};
pub use crate::cache::{Cache, CacheKey};
#[cfg(feature = "live-llm-cli")]
pub use crate::claude_cli::{ClaudeCliError, ClaudeCliLlm};
pub use crate::error::{LlmError, Result};
pub use crate::hashing::{canonical_profile_hash, compose_key, jd_hash};
pub use crate::mock::MockLlm;
pub use crate::retry::llm_backoff;
#[cfg(feature = "live-llm-api")]
pub use crate::rig::{Provider, RigLlm};
pub use crate::trait_def::Llm;
pub use crate::types::{LlmRequest, LlmResponse};

// Re-export the BackendChoice from core so callers don't need a separate
// import path.
pub use careerai_core::config::BackendChoice;
