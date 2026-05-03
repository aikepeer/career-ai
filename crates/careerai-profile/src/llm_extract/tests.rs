#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Mutex;

use async_trait::async_trait;

use super::build::build_request;
use super::parse::{extract_profile_from_text, parse_and_validate, strip_code_fences};
use super::types::{ExtractError, ExtractOptions, ExtractRequest, LlmCaller};

/// Minimal test double for [`LlmCaller`] — returns a queue of
/// canned responses in order. Errors out if the queue runs dry.
struct ScriptedLlm {
    queue: Mutex<Vec<Result<String, String>>>,
}

impl ScriptedLlm {
    fn new(responses: Vec<Result<String, String>>) -> Self {
        Self {
            queue: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl LlmCaller for ScriptedLlm {
    async fn call(&self, _req: &ExtractRequest) -> Result<String, String> {
        let mut q = self.queue.lock().unwrap();
        if q.is_empty() {
            return Err("scripted queue empty".to_string());
        }
        q.remove(0)
    }
}

fn valid_response() -> &'static str {
    r#"{
  "personal": {"name": "Alice Kumar", "email": "alice@example.com", "phone": "", "location": "Delhi", "links": {"github": "", "linkedin": "", "portfolio": ""}},
  "summary": "Builder of systems.",
  "skills": {"languages": ["Rust"], "frameworks": [], "tools": []},
  "experience": [{"title": "Senior Engineer", "company": "Acme", "location": "Remote", "start": "2022-01", "end": "Present", "bullets": ["Shipped things"]}],
  "education": [{"degree": "B.Tech", "institution": "IIT Delhi", "start": "2015", "end": "2019"}],
  "projects": []
}"#
}

#[test]
fn strip_fences_handles_json_marker() {
    let s = "```json\n{\"a\":1}\n```";
    assert_eq!(strip_code_fences(s), "{\"a\":1}");
}

#[test]
fn strip_fences_handles_bare_marker() {
    let s = "```\n{\"a\":1}\n```";
    assert_eq!(strip_code_fences(s), "{\"a\":1}");
}

#[test]
fn strip_fences_passthrough_when_clean() {
    let s = "{\"a\":1}";
    assert_eq!(strip_code_fences(s), "{\"a\":1}");
}

#[test]
fn parse_and_validate_accepts_minimal_profile() {
    let p = parse_and_validate(valid_response()).unwrap();
    assert_eq!(p.personal.name, "Alice Kumar");
    assert_eq!(p.experience.len(), 1);
}

#[test]
fn parse_and_validate_rejects_empty_name() {
    let bad = r#"{"personal":{"name":""},"summary":"","skills":{"languages":[],"frameworks":[],"tools":[]},"experience":[],"education":[],"projects":[]}"#;
    let err = parse_and_validate(bad).unwrap_err();
    assert!(matches!(err, ExtractError::SchemaValidate(_)));
}

#[test]
fn parse_and_validate_rejects_non_json() {
    let err = parse_and_validate("not json at all").unwrap_err();
    assert!(matches!(err, ExtractError::ParseJson(_)));
}

#[test]
fn parse_and_validate_distinguishes_shape_mismatch_from_invalid_json() {
    let bad = r#"{"personal":"Alice","summary":"","skills":{"languages":[],"frameworks":[],"tools":[]},"experience":[],"education":[],"projects":[]}"#;
    let err = parse_and_validate(bad).unwrap_err();
    assert!(
        matches!(err, ExtractError::SchemaDeserialize(_)),
        "expected SchemaDeserialize for shape mismatch, got {err:?}"
    );
}

#[tokio::test]
async fn extract_succeeds_on_first_attempt() {
    let llm = ScriptedLlm::new(vec![Ok(valid_response().to_string())]);
    let opts = ExtractOptions::default();
    let p = extract_profile_from_text("resume text", &llm, &opts)
        .await
        .unwrap();
    assert_eq!(p.personal.name, "Alice Kumar");
}

#[tokio::test]
async fn extract_recovers_on_retry() {
    let llm = ScriptedLlm::new(vec![
        Ok("not json".to_string()),
        Ok(valid_response().to_string()),
    ]);
    let p = extract_profile_from_text("resume text", &llm, &ExtractOptions::default())
        .await
        .unwrap();
    assert_eq!(p.personal.name, "Alice Kumar");
}

#[tokio::test]
async fn extract_gives_up_after_two_failures() {
    let llm = ScriptedLlm::new(vec![
        Ok("not json".to_string()),
        Ok("still not json".to_string()),
    ]);
    let err = extract_profile_from_text("resume text", &llm, &ExtractOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ExtractError::MaxRetries));
}

#[tokio::test]
async fn extract_surfaces_first_call_transport_error() {
    let llm = ScriptedLlm::new(vec![Err("rate limited".to_string())]);
    let err = extract_profile_from_text("resume text", &llm, &ExtractOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ExtractError::LlmCall(_)));
}

#[test]
fn request_build_is_deterministic() {
    let opts = ExtractOptions::default();
    let r1 = build_request("hello", &opts);
    let r2 = build_request("hello", &opts);
    assert_eq!(r1.system, r2.system);
    assert_eq!(r1.profile_block, r2.profile_block);
    assert_eq!(r1.user, r2.user);
}
