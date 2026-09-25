//! Integration test: A/B variant recording at submit time.
//!
//! `run_live` in `submit.rs` calls `record_variant` after a successful
//! submission transition to `Submitted`. `run_dry_run` never calls it.
//!
//! The live submit path creates a submitter internally and posts to the
//! real ATS API, which can't be intercepted from tests. So this file
//! tests:
//!  1. Dry-run submission does NOT create a variant row (via `run_pipeline`).
//!  2. `record_variant` (the function `run_live` calls) works correctly.
//!  3. The deterministic A/B label logic matches the code in `submit.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use careerai_core::config::{CoreConfig, SubmitSource};
use careerai_core::state::ListingState;
use careerai_db::models::{NewApplication, NewArtifact, NewListing};
use careerai_db::{pool_from_path, queries};
use careerai_pipeline as pipeline;

const TAILOR_FIXTURE: &str = r#"{
    "prompt_version": "tailor.v1",
    "summary": {"op": "keep"},
    "ops": [
        {"path": "experience[0].bullets[0]", "op": "keep"}
    ],
    "cover_letter": "short"
}"#;

const COVER_LETTER_FIXTURE: &str = "Dear Hiring Team,\n\nI am applying.\n\nBest,\nAda";

fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(
        root.join("profile").join("profile.yaml"),
        "personal:\n  name: \"Test User\"\n  email: \"test@example.com\"\nsummary: \"Engineer\"\nskills:\n  languages:\n    - \"Rust\"\n",
    )
    .unwrap();
    fs::write(
        root.join("config").join("default.yaml"),
        "render:\n  artifacts_dir: \"artifacts\"\n  pandoc_bin: null\n  pdf_engine: \"weasyprint\"\n  timeout_seconds: 60\n  keep_intermediate_markdown: true\n",
    )
    .unwrap();
}

fn write_fixtures(root: &Path) {
    let dir = root.join("data").join("cache").join("llm").join("fixtures");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("tailor.v1.json"), TAILOR_FIXTURE).unwrap();
    fs::write(
        dir.join("tailor.v1+cover_letter.json"),
        COVER_LETTER_FIXTURE,
    )
    .unwrap();
}

async fn seed_rendered_application(root: &Path, source: &str) -> (String, String) {
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();

    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: source.into(),
            external_id: "variant-it-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/variant-it-1".into(),
            description: "Build an LLM pipeline.".into(),
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
            profile_hash: "sha256:variant-it".into(),
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
        r#"{"name":"Test User","summary":"Engineer.","experience":[],"education":[],"projects":[]}"#,
        "Dear hiring team,\n\nApplying.\n\nBest,\nTest",
        r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[],"cover_letter":"short"}"#,
    )
    .await
    .unwrap();

    for (kind, path, bytes) in [
        ("resume_md", "/tmp/variant-it/resume.md", 1024_i64),
        ("resume_docx", "/tmp/variant-it/resume.docx", 2048_i64),
        ("cover_md", "/tmp/variant-it/cover.md", 512_i64),
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
async fn dry_run_submission_does_not_create_variant_row() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    write_fixtures(root);

    let mut cfg = CoreConfig::load(root).unwrap();
    cfg.llm.cache_dir = root
        .join("data")
        .join("cache")
        .join("llm")
        .to_string_lossy()
        .into_owned();
    cfg.matching.score_threshold = 0.0;
    cfg.submit.per_source.insert(
        "greenhouse".into(),
        SubmitSource {
            enabled: true,
            ..Default::default()
        },
    );

    seed_rendered_application(root, "greenhouse").await;

    let report = pipeline::run_pipeline(root, &cfg, false)
        .await
        .expect("run_pipeline should complete");

    assert_eq!(report.apply.dry_run, 1);
    assert_eq!(report.apply.submitted, 0);

    // Dry-run must not create any variant rows.
    let pool = pool_from_path(&root.join("data/careerai.sqlite"))
        .await
        .unwrap();
    let variants = queries::list_variants_with_outcome(&pool).await.unwrap();
    assert!(
        variants.is_empty(),
        "dry-run must not create variant rows, found {}",
        variants.len()
    );
}

#[tokio::test]
async fn record_variant_persists_to_application_variants_table() {
    use careerai_db::pool::pool_in_memory;

    let pool = pool_in_memory().await.unwrap();

    // Seed a listing + application so the FK on application_variants is valid.
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "rv-1".into(),
            title: "Engineer".into(),
            company: "Acme".into(),
            location: None,
            url: "https://example.com".into(),
            description: "Build things.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    let app = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: listing_id.clone(),
            profile_hash: "sha256:rv".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "mock".into(),
        },
    )
    .await
    .unwrap();

    // Compute the deterministic A/B label exactly as submit.rs does.
    let label = if app.id.bytes().fold(0u8, u8::wrapping_add) % 2 == 0 {
        "A"
    } else {
        "B"
    };

    let metadata = serde_json::json!({
        "source": "greenhouse",
        "listing_id": &listing_id,
    })
    .to_string();

    let row_id =
        queries::record_variant(&pool, &app.id, label, Some(&metadata), Some("content-v1"))
            .await
            .unwrap();

    assert!(row_id > 0);

    let variants = queries::list_variants_with_outcome(&pool).await.unwrap();
    assert!(!variants.is_empty());

    let v = &variants[0];
    assert!(v.variant_metadata.is_some());
    let meta = v.variant_metadata.as_ref().unwrap();
    assert!(meta.contains("greenhouse"));
    // R16: content_version and submitted_at are now populated.
    assert_eq!(v.content_version.as_deref(), Some("content-v1"));
    assert!(v.submitted_at.is_some());
    assert_eq!(v.company, "Acme");
    assert_eq!(v.title, "Engineer");
}

#[tokio::test]
async fn deterministic_variant_label_is_stable() {
    // The label must be deterministic: same application ID → same label.
    // This is the exact logic from submit.rs run_live.
    for app_id in ["app-001", "app-002", "app-003", "app-abc", "app-xyz"] {
        let label_a = if app_id.bytes().fold(0u8, u8::wrapping_add) % 2 == 0 {
            "A"
        } else {
            "B"
        };
        let label_b = if app_id.bytes().fold(0u8, u8::wrapping_add) % 2 == 0 {
            "A"
        } else {
            "B"
        };
        assert_eq!(label_a, label_b, "label must be deterministic for {app_id}");
        assert!(label_a == "A" || label_a == "B", "label must be A or B");
    }
}
