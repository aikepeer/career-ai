//! API retry and request-shape coverage using a local wiremock server.

#![cfg(feature = "live-llm-api")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use careerai_llm::{Llm, LlmRequest, Provider, RigLlm};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request() -> LlmRequest {
    LlmRequest {
        system: "system".into(),
        profile_block: "profile".into(),
        user: "user".into(),
        prompt_version: "test".into(),
        model: "test-model".into(),
        temperature: 0.0,
        max_tokens: 32,
        cache_profile: false,
    }
}

#[tokio::test]
async fn openai_retries_configured_transient_failures_and_sends_shape(
) -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("temporary"))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": "OK"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}
        })))
        .mount(&server)
        .await;

    let cache_dir = tempfile::tempdir()?;
    let llm = RigLlm::with_api_key_and_base_url_with_retries(
        Provider::OpenAI,
        "test-key".into(),
        "test-model",
        Some(server.uri()),
        Arc::new(careerai_llm::Cache::new(cache_dir.path())),
        1,
        2,
    )?;
    let response = llm.complete(&request()).await?;
    assert_eq!(response.text, "OK");
    assert_eq!(response.prompt_tokens, 3);
    assert_eq!(response.completion_tokens, 2);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let body: serde_json::Value = serde_json::from_slice(&requests[2].body)?;
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["messages"][0]["role"], "system");
    Ok(())
}

#[tokio::test]
async fn openai_zero_retries_makes_only_initial_request() -> Result<(), Box<dyn std::error::Error>>
{
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("temporary"))
        .mount(&server)
        .await;

    let cache_dir = tempfile::tempdir()?;
    let llm = RigLlm::with_api_key_and_base_url_with_retries(
        Provider::OpenAI,
        "test-key".into(),
        "test-model",
        Some(server.uri()),
        Arc::new(careerai_llm::Cache::new(cache_dir.path())),
        1,
        0,
    )?;
    assert!(llm.complete(&request()).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    Ok(())
}
