#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use super::driver::{Provider, RigLlm};
use super::error::is_retryable;
use crate::cache::Cache;
use crate::error::LlmError;
use crate::types::LlmRequest;

#[test]
fn is_retryable_classifies_correctly() {
    assert!(is_retryable(&LlmError::RateLimited {
        retry_after_seconds: 1
    }));
    assert!(is_retryable(&LlmError::Timeout { seconds: 1 }));
    assert!(is_retryable(&LlmError::Upstream("connection reset".into())));
    assert!(is_retryable(&LlmError::Upstream("http 503".into())));
    assert!(!is_retryable(&LlmError::Upstream("http 401".into())));
    assert!(!is_retryable(&LlmError::Schema("bad".into())));
}

#[test]
fn anthropic_body_attaches_cache_control_only_when_flagged() {
    let cache = Arc::new(Cache::new("/tmp/cache"));
    let llm = RigLlm::with_api_key(
        Provider::Anthropic,
        "sk-test".into(),
        "claude-3-5-sonnet-latest",
        cache,
        60,
    )
    .expect("construct");

    let mut req = LlmRequest {
        system: "sys".into(),
        profile_block: "PROFILE".into(),
        user: "USER".into(),
        prompt_version: "tailor.v1".into(),
        model: "claude-3-5-sonnet-latest".into(),
        temperature: 0.1,
        max_tokens: 1024,
        cache_profile: true,
    };
    let body = llm.anthropic_body(&req);
    let first_block = &body["messages"][0]["content"][0];
    assert_eq!(first_block["type"], "text");
    assert_eq!(first_block["text"], "PROFILE");
    assert_eq!(first_block["cache_control"]["type"], "ephemeral");

    req.cache_profile = false;
    let body = llm.anthropic_body(&req);
    assert!(body["messages"][0]["content"][0]
        .get("cache_control")
        .is_none());
}

#[test]
fn openai_body_shapes_system_and_user() {
    let cache = Arc::new(Cache::new("/tmp/cache"));
    let llm = RigLlm::with_api_key(Provider::OpenAI, "sk-test".into(), "gpt-4o-mini", cache, 60)
        .expect("construct");
    let req = LlmRequest {
        system: "sys".into(),
        profile_block: "PROFILE".into(),
        user: "USER".into(),
        prompt_version: "tailor.v1".into(),
        model: "gpt-4o-mini".into(),
        temperature: 0.2,
        max_tokens: 512,
        cache_profile: false,
    };
    let body = llm.openai_body(&req);
    let msgs = body["messages"].as_array().expect("messages array");
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "PROFILE");
    assert_eq!(msgs[2]["role"], "user");
    assert_eq!(msgs[2]["content"], "USER");
}
