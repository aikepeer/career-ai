//! Compile-only guard for the `live-llm` feature surface.
//!
//! The `anthropic_from_env_constructs` test is `#[ignore]` because it
//! needs a real `ANTHROPIC_API_KEY`; CI does not have one and should not
//! need one. `_assert_impl` is a zero-cost compile-time witness that
//! `RigLlm` still implements the [`Llm`] trait — if the trait method
//! signatures drift, this file fails to build under `--features live-llm`
//! before any runtime test would.
#![cfg(feature = "live-llm")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use careerai_llm::{Cache, Llm, Provider, RigLlm};

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn anthropic_from_env_constructs() {
    let cache = Arc::new(Cache::new("data/cache/llm"));
    let _rig =
        RigLlm::anthropic_from_env("claude-3-5-sonnet-latest", cache, 60).expect("construct");
}

#[test]
fn with_api_key_constructs_both_providers() {
    let cache = Arc::new(Cache::new("data/cache/llm"));
    let _a = RigLlm::with_api_key(
        Provider::Anthropic,
        "sk-test".into(),
        "claude-3-5-sonnet-latest",
        cache.clone(),
        60,
    )
    .expect("anthropic construct");
    let _o = RigLlm::with_api_key(Provider::OpenAI, "sk-test".into(), "gpt-4o-mini", cache, 60)
        .expect("openai construct");
}

#[allow(dead_code)]
fn _assert_impl(r: &RigLlm) -> &dyn Llm {
    r
}
