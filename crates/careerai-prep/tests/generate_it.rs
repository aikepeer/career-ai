use std::sync::Arc;

use async_trait::async_trait;
use careerai_core::config::CoreConfig;
use careerai_db::models::{NewApplication, NewListing};
use careerai_db::queries::{applications, listings};
use careerai_llm::error::Result as LlmResult;
use careerai_llm::trait_def::Llm;
use careerai_llm::types::{LlmRequest, LlmResponse};
use careerai_prep::generate_with_llm;

struct FixedLlm;

#[async_trait]
impl Llm for FixedLlm {
    fn name(&self) -> &'static str {
        "fixed-prep-test"
    }

    async fn complete(&self, _request: &LlmRequest) -> LlmResult<LlmResponse> {
        Ok(LlmResponse {
            text: r#"{
                "application_id": "ignored",
                "job_title": "ignored",
                "company": "ignored",
                "jd_summary": "Build Rust services.",
                "likely_topics": ["Rust"],
                "behavioral_questions": ["Describe a production incident."],
                "bullet_to_keyword": [],
                "company_news": [],
                "generated_at": "2026-01-01T00:00:00Z"
            }"#
            .into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cache_hit: false,
            cached_prompt_tokens: 0,
        })
    }
}

#[tokio::test]
async fn generate_writes_grounded_markdown_for_application() {
    let root = tempfile::tempdir().unwrap();
    let db_path = careerai_core::paths::database_path(root.path());
    let pool = careerai_db::pool_from_path(&db_path).await.unwrap();
    let listing_id = listings::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "prep-1".into(),
            title: "Rust Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://acme.example/jobs/prep-1".into(),
            description: "Build Rust services".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap()
    .0;
    let application = applications::create_application(
        &pool,
        &NewApplication {
            listing_id,
            profile_hash: "sha256:test".into(),
            prompt_version: "test".into(),
            llm_model: "fixed".into(),
        },
    )
    .await
    .unwrap();
    let profile_path = careerai_core::paths::profile_path(root.path());
    std::fs::create_dir_all(profile_path.parent().unwrap()).unwrap();
    std::fs::write(
        profile_path,
        "personal:\n  name: Test Candidate\nsummary: Rust engineer\nexperience:\n  - title: Engineer\n    company: Acme\n    start: '2020'\n    end: present\n    bullets:\n      - Built Rust services\n",
    )
    .unwrap();
    let llm = Arc::new(FixedLlm);

    let path = generate_with_llm(
        root.path(),
        &CoreConfig::load(root.path()).unwrap(),
        &application.id,
        llm.as_ref(),
        &[],
    )
    .await
    .unwrap();
    let markdown = std::fs::read_to_string(path).unwrap();
    assert!(markdown.contains("# Interview prep — Rust Engineer @ Acme"));
    assert!(markdown.contains("Build Rust services."));
    assert!(markdown.contains("## Talking points"));
}
