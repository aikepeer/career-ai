//! Integration test for the LLM-backed extractor. Drives
//! [`extract_profile_from_text`] with a scripted in-memory `LlmCaller`
//! that returns canned JSON; no network. Snapshots the resulting
//! [`Profile`] so we notice if the schema or merge contract drifts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use careerai_profile::{
    extract_profile_from_text, llm_extract::ExtractRequest, ExtractOptions, LlmCaller, Profile,
};

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

const CANNED_RESPONSE: &str = r#"{
  "personal": {
    "name": "Alice Kumar",
    "email": "alice@example.com",
    "phone": "+91 98765 43210",
    "location": "Delhi NCR",
    "links": {
      "github": "https://github.com/alicek",
      "linkedin": "https://linkedin.com/in/alicek",
      "portfolio": ""
    }
  },
  "summary": "Senior embedded developer with 8 years building real-time control loops on STM32 and Linux ARM. Recently shipped an LLM-powered debug assistant that cut on-call MTTR by 40%.",
  "skills": {
    "languages": ["Rust", "C", "C++", "Python"],
    "frameworks": ["Tokio", "Zephyr", "FreeRTOS"],
    "tools": ["Git", "Docker", "JTAG", "CAN bus analyzers"]
  },
  "experience": [
    {
      "title": "Senior Embedded Developer",
      "company": "Acme Robotics",
      "location": "Remote",
      "start": "2022-01",
      "end": "Present",
      "bullets": [
        "Led migration of motor-control firmware from FreeRTOS to Zephyr, cutting binary size 30% and boot time 2x.",
        "Designed CAN bus arbitration scheme adopted across three product lines."
      ]
    },
    {
      "title": "Embedded Engineer",
      "company": "BetaCorp",
      "location": "Bangalore",
      "start": "2019-06",
      "end": "2021-12",
      "bullets": [
        "Built ingest pipeline processing 12k events/sec from on-vehicle telemetry.",
        "Mentored two junior engineers through firmware bring-up."
      ]
    }
  ],
  "education": [
    {
      "degree": "B.Tech, Computer Science Engineering",
      "institution": "IIT Delhi",
      "start": "2015",
      "end": "2019"
    }
  ],
  "projects": [
    {
      "name": "embedded-llm-helper",
      "url": "https://github.com/alicek/embedded-llm-helper",
      "bullets": ["On-device tinyllama for firmware debugging on Cortex-M7."]
    }
  ]
}"#;

#[tokio::test]
async fn extracts_profile_from_fixture_resume_text() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("resume_text.txt");
    let text = std::fs::read_to_string(&fixture).expect("read fixture");

    let llm = ScriptedLlm::new(vec![Ok(CANNED_RESPONSE.to_string())]);
    let profile = extract_profile_from_text(&text, &llm, &ExtractOptions::default())
        .await
        .expect("extract");

    profile.check().expect("schema validates");
    insta::assert_yaml_snapshot!("extracts_profile_from_fixture", profile);
}

#[tokio::test]
async fn extractor_recovers_when_first_response_has_code_fence() {
    // Real Claude responses often wrap JSON in ```json fences even when
    // the prompt forbids it. The extractor must strip the fences and
    // succeed on the first attempt.
    let fenced = format!("```json\n{CANNED_RESPONSE}\n```");
    let llm = ScriptedLlm::new(vec![Ok(fenced)]);
    let profile: Profile = extract_profile_from_text(
        "ignored, fixture not needed for this case",
        &llm,
        &ExtractOptions::default(),
    )
    .await
    .expect("extract");
    assert_eq!(profile.personal.name, "Alice Kumar");
    assert_eq!(profile.experience.len(), 2);
}
