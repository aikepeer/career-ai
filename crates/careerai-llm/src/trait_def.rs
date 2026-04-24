//! The `Llm` trait.
//!
//! Mirrors the `Source` trait pattern at
//! `crates/careerai-sources/src/base.rs:36-46`: async_trait, `Send + Sync`,
//! a stable `name()` plus the one-method async workhorse. Adding a new LLM
//! provider means implementing this trait; `careerai-core` never branches
//! on concrete provider types.

use async_trait::async_trait;

use crate::error::Result;
use crate::types::{LlmRequest, LlmResponse};

#[async_trait]
pub trait Llm: Send + Sync {
    /// Stable identifier: `"mock"`, `"rig"`, etc. Used for logs and cache
    /// keys if we ever partition by provider.
    fn name(&self) -> &'static str;

    /// Issue a completion. Implementations own their own retry + cache
    /// policy; callers pass a fully-formed `LlmRequest`.
    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse>;
}
