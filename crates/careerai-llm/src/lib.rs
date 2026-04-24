//! LLM gateway.
//!
//! Single call site for all LLM traffic. Provider-agnostic `Llm` trait with
//! `MockLlm` as the default deterministic impl; `rig-core`-backed provider
//! lives behind the `live-llm` feature (Wave 3B). On-disk response cache
//! keyed by sha256 of (prompt_version, profile_hash, jd_hash, model).

#![forbid(unsafe_code)]

pub mod cache;
pub mod error;
pub mod hashing;
pub mod mock;
pub mod retry;
#[cfg(feature = "live-llm")]
pub mod rig;
pub mod trait_def;
pub mod types;

pub use crate::cache::{Cache, CacheKey};
pub use crate::error::{LlmError, Result};
pub use crate::hashing::{canonical_profile_hash, compose_key, jd_hash};
pub use crate::mock::MockLlm;
pub use crate::retry::llm_backoff;
#[cfg(feature = "live-llm")]
pub use crate::rig::{Provider, RigLlm};
pub use crate::trait_def::Llm;
pub use crate::types::{LlmRequest, LlmResponse};
