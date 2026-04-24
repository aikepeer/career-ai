//! Integration tests for `MockLlm`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_llm::{Llm, LlmError, LlmRequest, MockLlm};
use tempfile::tempdir;

fn req(prompt_version: &str) -> LlmRequest {
    LlmRequest {
        system: "sys".into(),
        profile_block: "profile".into(),
        user: "user".into(),
        prompt_version: prompt_version.into(),
        model: "mock-1".into(),
        temperature: 0.0,
        max_tokens: 256,
        cache_profile: false,
    }
}

#[tokio::test]
async fn with_fixture_returns_canned_text() {
    let llm = MockLlm::with_fixture("tailor.v1", "canned-response");
    let got = llm.complete(&req("tailor.v1")).await.unwrap();
    assert_eq!(got.text, "canned-response");
    assert!(!got.cache_hit);
    assert_eq!(got.prompt_tokens, 0);
}

#[tokio::test]
async fn missing_fixture_reports_prompt_version_in_error() {
    let llm = MockLlm::new();
    let err = llm.complete(&req("cover.v1")).await.unwrap_err();
    match err {
        LlmError::Upstream(msg) => {
            assert!(
                msg.contains("cover.v1"),
                "error should name the missing prompt_version, got: {msg}"
            );
        }
        other => panic!("expected Upstream, got {other:?}"),
    }
}

#[tokio::test]
async fn from_dir_loads_multiple_fixtures() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("tailor.v1.json"), "TAILOR-BODY").unwrap();
    std::fs::write(dir.path().join("cover.v1.json"), "COVER-BODY").unwrap();
    // Non-json file should be ignored.
    std::fs::write(dir.path().join("README.txt"), "ignored").unwrap();

    let llm = MockLlm::from_dir(dir.path()).unwrap();
    assert_eq!(
        llm.complete(&req("tailor.v1")).await.unwrap().text,
        "TAILOR-BODY"
    );
    assert_eq!(
        llm.complete(&req("cover.v1")).await.unwrap().text,
        "COVER-BODY"
    );
    assert!(llm.complete(&req("nope.v1")).await.is_err());
}

#[tokio::test]
async fn name_is_mock() {
    let llm = MockLlm::new();
    assert_eq!(llm.name(), "mock");
}
