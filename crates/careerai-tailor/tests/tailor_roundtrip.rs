//! Golden snapshot of `tailor_for_listing` on an in-memory SQLite pool
//! with a canned LLM fixture. Verifies the `ResumeView` after the
//! constrained diff applies cleanly — including reorder + reword.

mod common;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::common::fixture_profile;
    use careerai_core::config::LlmConfig;
    use careerai_db::models::NewListing;
    use careerai_db::pool::pool_in_memory;
    use careerai_db::queries;
    use careerai_llm::mock::MockLlm;
    use careerai_tailor::tailor_for_listing;

    fn fixture_cfg() -> LlmConfig {
        LlmConfig {
            provider: String::new(),
            model: String::new(),
            tailor_model: "mock-model".into(),
            cover_letter_model: "mock-model".into(),
            filter_model: String::new(),
            parse_resume_model: String::new(),
            cache_dir: String::new(),
            api_base_url: None,
            api_key: None,
            max_retries: 3,
            timeout_seconds: 120,
            prompt_version: "tailor.v1".into(),
            anthropic_prompt_cache: true,
            backend: careerai_core::config::BackendChoice::default(),
        }
    }

    const CANNED_TAILOR_JSON: &str = r#"{
        "prompt_version":"tailor.v1",
        "summary":{"op":"keep"},
        "ops":[
            {"path":"experience[0].bullets[1]","op":"move_before","target_path":"experience[0].bullets[0]"},
            {"path":"experience[0].bullets[0]","op":"reword","new_text":"Shipped Rust LLM pipeline cutting latency 35%."},
            {"path":"experience[1].bullets[0]","op":"keep"},
            {"path":"projects[0].bullets[0]","op":"keep"}
        ],
        "cover_letter":"I build things."
    }"#;

    const CANNED_COVER_LETTER: &str = "Dear Hiring Team,\n\nI am applying.\n\nRegards.";

    #[tokio::test]
    async fn tailor_for_listing_produces_expected_resume_view() {
        // Set cache_dir to an ephemeral path so puts don't hit real disk.
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = fixture_cfg();
        cfg.cache_dir = tmp.path().to_string_lossy().into_owned();

        let pool = pool_in_memory().await.unwrap();

        // Seed a listing so find_by_id works.
        let new = NewListing {
            source: "fixture".into(),
            external_id: "rt-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/rt-1".into(),
            description: "Build and ship a real-time LLM system.".into(),
            raw_json: None,
        };
        let (listing_id, _) = queries::insert_or_ignore(&pool, &new).await.unwrap();
        // Move to Shortlisted so the Tailored transition is legal in the
        // state machine. (The tailor impl itself doesn't gate on prior
        // state; this mirrors the real pipeline.)
        queries::transition(
            &pool,
            &listing_id,
            careerai_core::state::ListingState::Shortlisted,
            None,
        )
        .await
        .unwrap();

        // Build a MockLlm with both fixtures — one per prompt_version.
        let mut llm = MockLlm::new();
        llm.insert_fixture("tailor.v1", CANNED_TAILOR_JSON);
        llm.insert_fixture("tailor.v1+cover_letter", CANNED_COVER_LETTER);

        let profile = fixture_profile();
        let outcome = tailor_for_listing(&pool, &llm, &listing_id, &profile, &cfg)
            .await
            .unwrap();

        // Golden snapshot.
        insta::assert_yaml_snapshot!("resume_view", outcome.resume_view);

        // Application row + listing state.
        let app = queries::find_application_by_id(&pool, &outcome.application_id)
            .await
            .unwrap();
        assert_eq!(app.state, "tailored");
        assert_eq!(app.llm_model, "mock-model");
        assert_eq!(app.prompt_version, "tailor.v1");

        let listing = queries::find_by_id(&pool, &listing_id).await.unwrap();
        assert_eq!(listing.state, "tailored");

        assert_eq!(outcome.cover_letter.body, CANNED_COVER_LETTER);
    }
}
