//! Regression tests for `retry_application` — a failed application must
//! be resettable to its pre-submit state and no longer stuck in `failed`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::models::{NewApplication, NewArtifact, NewListing};
use careerai_db::{pool_from_path, queries};
use careerai_pipeline as pipeline;
use careerai_profile::schema::{Personal, Profile, Skills};

fn fixture_profile() -> Profile {
    Profile {
        personal: Personal {
            name: "Ada Lovelace".into(),
            email: "ada@example.com".into(),
            phone: "555-0101".into(),
            ..Default::default()
        },
        summary: "Senior Rust engineer.".into(),
        skills: Skills {
            languages: vec!["Rust".into()],
            ..Default::default()
        },
        ..Default::default()
    }
}

fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    let yaml = fixture_profile().to_yaml().unwrap();
    fs::write(root.join("profile").join("profile.yaml"), yaml).unwrap();
    fs::write(
        root.join("config").join("default.yaml"),
        "render:\n  artifacts_dir: \"artifacts\"\n",
    )
    .unwrap();
}

/// Seed a rendered greenhouse application, then flip it to `failed` the
/// same way a live submission error does (application + listing + event).
async fn seed_failed_application(root: &Path) -> (String, String) {
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();

    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "retry-it-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/retry-it-1".into(),
            description: "Build and ship an LLM pipeline.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    for to in [
        ListingState::Shortlisted,
        ListingState::Tailored,
        ListingState::Rendered,
    ] {
        queries::transition(&pool, &listing_id, to, None)
            .await
            .unwrap();
    }

    let app = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: listing_id.clone(),
            profile_hash: "sha256:retry-it".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "mock".into(),
        },
    )
    .await
    .unwrap();
    queries::set_application_state(&pool, &app.id, "rendered")
        .await
        .unwrap();

    queries::write_payload(
        &pool,
        &app.id,
        r#"{"name":"Ada Lovelace","summary":"Senior Rust engineer.","experience":[],"education":[],"projects":[]}"#,
        "Dear hiring team,\n\nApplying for the role.\n\nBest,\nAda",
        r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[],"cover_letter":"short"}"#,
    )
    .await
    .unwrap();

    for (kind, path, bytes) in [
        ("resume_md", "/tmp/retry-it/resume.md", 1024_i64),
        ("resume_docx", "/tmp/retry-it/resume.docx", 2048_i64),
        ("cover_md", "/tmp/retry-it/cover.md", 512_i64),
    ] {
        queries::attach_artifact(
            &pool,
            &app.id,
            &NewArtifact {
                kind: kind.into(),
                path: path.into(),
                bytes,
            },
        )
        .await
        .unwrap();
    }

    // Mirror the live-submission failure path: both rows go to failed and
    // an event records rendered → failed.
    queries::transition_application_and_listing(
        &pool,
        &app.id,
        &listing_id,
        ListingState::Failed.as_str(),
        ListingState::Failed,
        Some("failed: simulated submission error"),
    )
    .await
    .unwrap();

    drop(pool);
    (listing_id, app.id)
}

#[tokio::test]
async fn retry_application_leaves_failed_state() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    let (listing_id, application_id) = seed_failed_application(root).await;

    let cfg = CoreConfig::load(root).unwrap();

    let outcome = pipeline::retry_application(root, &cfg, &application_id)
        .await
        .expect("retry_application should succeed");

    // The application must no longer be failed. Dry-run leaves it rendered;
    // a live/skip path would also leave a non-failed state, so the exact
    // outcome is not the point — the stuck state is.
    let pool = pool_from_path(&root.join("data/careerai.sqlite"))
        .await
        .unwrap();
    let app = queries::find_application_by_id(&pool, &application_id)
        .await
        .unwrap();
    assert_ne!(app.state, "failed", "application is still stuck in failed");

    let listing = queries::find_by_id(&pool, &listing_id).await.unwrap();
    assert_ne!(listing.state, "failed", "listing is still stuck in failed");

    // The retry actually re-ran apply_one: dry-run preserves rendered, and
    // the seeded source (greenhouse) is enabled by default with
    // auto_submit=false.
    assert!(
        matches!(
            &outcome.outcome,
            careerai_submit::SubmitOutcome::DryRun { .. }
        ),
        "expected dry-run retry, got {:?}",
        outcome.outcome,
    );
}
