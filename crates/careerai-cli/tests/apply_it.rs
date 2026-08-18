//! Integration coverage for `pipeline::apply_one`, `apply_all`,
//! `applied_show`, and `inspect_show`.
//!
//! Stays fully offline: ATS HTTP `submit()` bodies are covered by the submit
//! crate's wiremock integration tests, so here forcing `--auto-submit`
//! against an unknown source surfaces the submit-layer `UnknownSource` error
//! without ever reaching the network. Dry-run path exercises the
//! `DryRunSubmitter` wrapper end-to-end via the real `submit_application`
//! entry point.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use std::fs;
use std::path::Path;

use careerai_core::config::{CoreConfig, SubmitSource};
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

/// Scaffold the minimum directory tree + profile YAML the submit path
/// expects under `root`.
fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    let yaml = fixture_profile().to_yaml().unwrap();
    fs::write(root.join("profile").join("profile.yaml"), yaml).unwrap();
    // Minimal config override — loader merges with embedded defaults.
    fs::write(
        root.join("config").join("default.yaml"),
        "render:\n  artifacts_dir: \"artifacts\"\n",
    )
    .unwrap();
}

/// Seed a listing already transitioned to `rendered`, an application in the
/// same state, its payload row, and a couple of artifacts. Returns
/// `(listing_id, application_id)`.
async fn seed_rendered_application(root: &Path, source: &str) -> (String, String) {
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();

    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: source.into(),
            external_id: "apply-it-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/apply-it-1".into(),
            description: "Build and ship an LLM pipeline.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    // Walk the listing to 'rendered' so the submit invariant is satisfied.
    for to in [
        careerai_core::state::ListingState::Shortlisted,
        careerai_core::state::ListingState::Tailored,
        careerai_core::state::ListingState::Rendered,
    ] {
        queries::transition(&pool, &listing_id, to, None)
            .await
            .unwrap();
    }

    let app = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: listing_id.clone(),
            profile_hash: "sha256:apply-it".into(),
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
        ("resume_md", "/tmp/apply-it/resume.md", 1024_i64),
        ("resume_docx", "/tmp/apply-it/resume.docx", 2048_i64),
        ("cover_md", "/tmp/apply-it/cover.md", 512_i64),
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

    drop(pool);
    (listing_id, app.id)
}

#[tokio::test]
async fn apply_one_dry_run_preserves_state() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    let (listing_id, application_id) = seed_rendered_application(root, "greenhouse").await;

    let mut cfg = CoreConfig::load(root).expect("load cfg");
    // Opt greenhouse in — dry-run still runs with per_source enabled; we
    // want to prove dry-run wins even when the source is live-capable.
    cfg.submit.per_source.insert(
        "greenhouse".into(),
        SubmitSource {
            enabled: true,
            ..Default::default()
        },
    );

    let outcome = pipeline::apply_one(root, &cfg, &application_id, Some(false))
        .await
        .expect("apply_one dry-run ok");

    assert_eq!(outcome.application_id, application_id);
    assert_eq!(outcome.source, "greenhouse");
    match &outcome.outcome {
        careerai_submit::SubmitOutcome::DryRun { payload_summary } => {
            assert!(
                payload_summary.contains("POST"),
                "unexpected dry-run summary: {payload_summary}"
            );
        }
        other => panic!("expected DryRun, got {other:?}"),
    }

    // State must be unchanged by dry-run: application stays 'rendered',
    // listing stays 'rendered'.
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();
    let app = queries::find_application_by_id(&pool, &application_id)
        .await
        .unwrap();
    assert_eq!(
        app.state, "rendered",
        "dry-run must not transition application"
    );
    let listing = queries::find_by_id(&pool, &listing_id).await.unwrap();
    assert_eq!(
        listing.state, "rendered",
        "dry-run must not transition listing"
    );
}

#[tokio::test]
async fn apply_one_force_live_surfaces_unknown_source_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    let (_listing_id, application_id) = seed_rendered_application(root, "unknown").await;

    let mut cfg = CoreConfig::load(root).expect("load cfg");
    // per_source must be enabled, otherwise the gate would convert the
    // call to Skipped before ever reaching the submitter dispatch.
    cfg.submit.per_source.insert(
        "unknown".into(),
        SubmitSource {
            enabled: true,
            ..Default::default()
        },
    );

    let err = pipeline::apply_one(root, &cfg, &application_id, Some(true))
        .await
        .expect_err("live submit must surface UnknownSource from the submit layer");

    // The submit layer rejects unknown sources before any network call;
    // submit_application propagates that typed error through the pipeline.
    let msg = err.to_string().to_lowercase();
    assert!(
        err.chain().any(|c| c
            .downcast_ref::<careerai_submit::SubmitError>()
            .is_some_and(|e| matches!(e, careerai_submit::SubmitError::UnknownSource(_)))),
        "expected UnknownSource in cause chain; got: {msg}"
    );
}

#[tokio::test]
async fn applied_show_is_empty_after_dry_run_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    let (_l, application_id) = seed_rendered_application(root, "greenhouse").await;

    let mut cfg = CoreConfig::load(root).expect("load cfg");
    cfg.submit.per_source.insert(
        "greenhouse".into(),
        SubmitSource {
            enabled: true,
            ..Default::default()
        },
    );

    // Drive a dry-run so something has happened.
    let _ = pipeline::apply_one(root, &cfg, &application_id, Some(false))
        .await
        .unwrap();

    let rows = pipeline::applied_show(root, None, 20).await.unwrap();
    assert!(
        rows.is_empty(),
        "dry-run must not leave a submitted row (got {} rows)",
        rows.len()
    );
}

#[tokio::test]
async fn inspect_show_surfaces_events_and_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    let (_listing_id, application_id) = seed_rendered_application(root, "greenhouse").await;

    let report = pipeline::inspect_show(root, &application_id).await.unwrap();
    assert_eq!(report.application.id, application_id);
    assert_eq!(report.listing_source, "greenhouse");
    assert_eq!(report.listing_company, "Beta Corp");

    // Events: seed walks discovered → shortlisted → tailored → rendered.
    let states: Vec<&str> = report.events.iter().map(|e| e.to_state.as_str()).collect();
    for expected in ["discovered", "shortlisted", "tailored", "rendered"] {
        assert!(
            states.contains(&expected),
            "missing '{expected}' event; saw: {states:?}"
        );
    }

    // Artifacts: seeded 3.
    assert_eq!(report.artifacts.len(), 3);
    let kinds: Vec<&str> = report.artifacts.iter().map(|a| a.kind.as_str()).collect();
    assert!(kinds.contains(&"resume_docx"));
}
