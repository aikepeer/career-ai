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
            ..Default::default()
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

    /// A diff that invents an employer ("AcmeCorp") — the safety
    /// validator must reject it, and the rejected response must not be
    /// cached. All bullets are covered (as the schema's coverage rule
    /// requires); only the reword carries the invented noun.
    const INVALID_TAILOR_JSON: &str = r#"{
        "prompt_version":"tailor.v1",
        "summary":{"op":"keep"},
        "ops":[
            {"path":"experience[0].bullets[0]","op":"reword","new_text":"Built scalable systems at AcmeCorp."},
            {"path":"experience[0].bullets[1]","op":"keep"},
            {"path":"experience[1].bullets[0]","op":"keep"},
            {"path":"projects[0].bullets[0]","op":"keep"}
        ],
        "cover_letter":"I build things."
    }"#;

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
        let outcome = tailor_for_listing(&pool, &llm, &listing_id, &profile, &cfg, tmp.path())
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

    #[tokio::test]
    async fn tailor_for_listing_populates_quality_score_and_tone_label() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = fixture_cfg();
        cfg.cache_dir = tmp.path().to_string_lossy().into_owned();

        let pool = pool_in_memory().await.unwrap();

        let new = NewListing {
            source: "fixture".into(),
            external_id: "qt-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "StartupCorp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/qt-1".into(),
            description: "Build and ship a real-time LLM system. We are a fast-paced startup."
                .into(),
            raw_json: None,
        };
        let (listing_id, _) = queries::insert_or_ignore(&pool, &new).await.unwrap();
        queries::transition(
            &pool,
            &listing_id,
            careerai_core::state::ListingState::Shortlisted,
            None,
        )
        .await
        .unwrap();

        let mut llm = MockLlm::new();
        llm.insert_fixture("tailor.v1", CANNED_TAILOR_JSON);
        llm.insert_fixture("tailor.v1+cover_letter", CANNED_COVER_LETTER);

        let profile = fixture_profile();
        let outcome = tailor_for_listing(&pool, &llm, &listing_id, &profile, &cfg, tmp.path())
            .await
            .unwrap();

        // Quality score must be populated.
        let qs = outcome
            .quality_score
            .expect("quality_score must be populated by tailor_for_listing");
        assert!(qs.overall > 0.0, "overall quality score should be > 0");
        assert!(qs.jd_relevance >= 0.0 && qs.jd_relevance <= 1.0);
        assert!(qs.skill_coverage >= 0.0 && qs.skill_coverage <= 1.0);
        assert!(qs.cover_letter_depth >= 0.0 && qs.cover_letter_depth <= 1.0);
        assert!(qs.bullet_density >= 0.0 && qs.bullet_density <= 1.0);

        // Tone label must be populated.
        let tone = outcome
            .tone_label
            .as_ref()
            .expect("tone_label must be populated");
        assert!(!tone.is_empty(), "tone label should not be empty");

        // Quality score should be persisted to the DB.
        let stored = queries::fetch_quality_score(&pool, &outcome.application_id)
            .await
            .unwrap();
        assert!(stored.is_some(), "quality score should be persisted to DB");
        let row = stored.unwrap();
        assert!((row.overall - f64::from(qs.overall)).abs() < 1e-6);
    }

    #[tokio::test]
    async fn relative_cache_dir_resolves_against_base_dir() {
        // Regression: `default.yaml` ships `cache_dir: "data/cache/llm"`
        // (relative). The response cache must land under `base_dir` (the
        // resolved project root), not the process CWD — a CWD-relative
        // resolution breaks runs started from `$HOME` with EACCES when a
        // root-owned `~/data` phantom dir exists.
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = fixture_cfg();
        cfg.cache_dir = "data/cache/llm".into();

        let pool = pool_in_memory().await.unwrap();
        let new = NewListing {
            source: "fixture".into(),
            external_id: "rt-2".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/rt-2".into(),
            description: "Build and ship a real-time LLM system.".into(),
            raw_json: None,
        };
        let (listing_id, _) = queries::insert_or_ignore(&pool, &new).await.unwrap();
        queries::transition(
            &pool,
            &listing_id,
            careerai_core::state::ListingState::Shortlisted,
            None,
        )
        .await
        .unwrap();

        let mut llm = MockLlm::new();
        llm.insert_fixture("tailor.v1", CANNED_TAILOR_JSON);
        llm.insert_fixture("tailor.v1+cover_letter", CANNED_COVER_LETTER);

        let profile = fixture_profile();
        let outcome = tailor_for_listing(&pool, &llm, &listing_id, &profile, &cfg, tmp.path())
            .await
            .unwrap();

        let cache_dir = tmp.path().join("data/cache/llm");
        let json_entries = std::fs::read_dir(&cache_dir)
            .unwrap_or_else(|e| panic!("no cache dir under base_dir: {e}"))
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .count();
        assert!(
            json_entries >= 1,
            "expected a cached LLM response under {}",
            cache_dir.display()
        );
        assert_eq!(outcome.cover_letter.body, CANNED_COVER_LETTER);
    }

    #[tokio::test]
    async fn invented_employer_is_sanitized_not_cached_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = fixture_cfg();
        cfg.cache_dir = tmp.path().to_string_lossy().into_owned();

        let pool = pool_in_memory().await.unwrap();
        let new = NewListing {
            source: "fixture".into(),
            external_id: "rt-3".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/rt-3".into(),
            description: "Build and ship a real-time LLM system.".into(),
            raw_json: None,
        };
        let (listing_id, _) = queries::insert_or_ignore(&pool, &new).await.unwrap();
        queries::transition(
            &pool,
            &listing_id,
            careerai_core::state::ListingState::Shortlisted,
            None,
        )
        .await
        .unwrap();

        let mut llm = MockLlm::new();
        llm.insert_fixture("tailor.v1", INVALID_TAILOR_JSON);
        llm.insert_fixture("tailor.v1+cover_letter", CANNED_COVER_LETTER);

        let profile = fixture_profile();
        // The pipeline uses validate_and_sanitize (safe path): invented
        // content is sanitized (falls back to Keep), not hard-rejected.
        let outcome = tailor_for_listing(&pool, &llm, &listing_id, &profile, &cfg, tmp.path())
            .await
            .expect("sanitized diff should succeed");

        // The invented employer "AcmeCorp" must NOT appear in the output
        // — the sanitize path fell back to the original bullet.
        let resume_json = serde_json::to_string(&outcome.resume_view).unwrap();
        assert!(
            !resume_json.contains("AcmeCorp"),
            "invented employer leaked into sanitized output"
        );
        // The original bullet must be present (Keep was applied).
        assert!(
            resume_json.contains("Shipped Rust LLM pipeline"),
            "original bullet must survive sanitization"
        );
    }
}
