//! Provider-agnostic DTOs for an LLM call.
//!
//! `LlmRequest::profile_block` is kept as a separate segment so Anthropic
//! prompt-caching can attach `cache_control: {type:"ephemeral"}` only to
//! that segment (wired in the `live-llm` feature). `LlmResponse::cache_hit`
//! reports the on-disk cache (our cache), distinct from
//! `cached_prompt_tokens` which reports Anthropic's own prompt cache reuse.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub system: String,
    /// Cacheable block under Anthropic prompt caching. Typically the
    /// serialized master profile. Kept separate from `user` so only this
    /// segment gets `cache_control` attached.
    pub profile_block: String,
    pub user: String,
    pub prompt_version: String,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// When true, the provider impl should attach prompt-cache metadata to
    /// `profile_block`. Providers without prompt-cache support must ignore.
    pub cache_profile: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// True when served from the on-disk `Cache`; false on live provider calls.
    pub cache_hit: bool,
    /// Prompt tokens reported by the provider as served from *their* cache
    /// (e.g. Anthropic prompt caching). Independent of `cache_hit`.
    pub cached_prompt_tokens: u32,
}
