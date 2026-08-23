//! Integration tests for `run_pipeline` — the full shortlist → tailor →
//! render → apply orchestration behind `careerai run` and the dashboard's
//! "Run All" control.
//!
//! Stays offline: tailoring uses `MockLlm` fixtures, apply uses the
//! dry-run / per-source-gate / LinkedIn-draft paths, and no adapter ever
//! hits the network. The render half is gated on `pandoc` + a PDF engine
//! exactly like `careerai-cli/tests/tailor_render_it.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use std::fs;
use std::path::Path;

use careerai_core::config::{CoreConfig, SubmitSource};
use careerai_core::state::ListingState;
use careerai_db::models::{NewApplication, NewArtifact, NewListing};
use careerai_db::{pool_from_path, queries};
use careerai_pipeline as pipeline;
use careerai_profile::schema::{Education, Experience, Personal, Profile, Project, Skills};

const TAILOR_FIXTURE: &str = r#"{
    "prompt_version": "tailor.v1",
    "summary": {"op": "keep"},
    "ops": [
        {"path": "experience[0].bullets[0]", "op": "keep"},
        {"path": "experience[0].bullets[1]", "op": "keep"},
        {"path": "experience[1].bullets[0]", "op": "keep"},
        {"path": "experience[1].bullets[1]", "op": "keep"},
        {"path": "projects[0].bullets[0]", "op": "keep"}
    ],
    "cover_letter": "short"
}"#;

const COVER_LETTER_FIXTURE: &str = "Dear Hiring Team,\n\nI am applying for the role.\n\nBest,\nAda";

fn fixture_profile() -> Profile {
    Profile {
        personal: Personal {
            name: "Ada Lovelace".into(),
            email: "ada@example.com".into(),
            ..Default::default()
        },
        summary: "Senior Rust engineer building LLM + robotics systems.".into(),
        skills: Skills {
            languages: vec!["Rust".into(), "Python".into()],
            frameworks: vec!["Tokio".into()],
            tools: vec!["SQLite".into()],
            ..Default::default()
        },
        experience: vec![
            Experience {
                title: "Senior Engineer".into(),
                company: "Acme Robotics".into(),
                location: "Remote".into(),
                start: "2022-01".into(),
                end: "present".into(),
                bullets: vec![
                    "Shipped Rust LLM pipeline reducing latency 35%.".into(),
                    "Led team of 4 on embedded perception.".into(),
                ],
            },
            Experience {
                title: "Engineer".into(),
                company: "Widget Corp".into(),
                location: String::new(),
                start: "2018-06".into(),
                end: "2021-12".into(),
                bullets: vec![
                    "Built Python services on Kubernetes.".into(),
                    "Migrated legacy Java to Go.".into(),
                ],
            },
        ],
        education: vec![Education {
            degree: "BSc Mathematics".into(),
            institution: "Analytical Engine University".into(),
            start: "2012".into(),
            end: "2016".into(),
            ..Default::default()
        }],
        projects: vec![Project {
            name: "openLLM".into(),
            url: "https://example.com/openllm".into(),
            bullets: vec!["Tokenizer in Rust supporting 5 languages.".into()],
        }],
    }
}

fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    let profile_yaml = fixture_profile().to_yaml().unwrap();
    fs::write(root.join("profile").join("profile.yaml"), profile_yaml).unwrap();
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

async fn seed_listing(root: &Path, source: &str, state: ListingState) -> String {
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: source.into(),
            external_id: "run-it-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/run-it-1".into(),
            description: "Build and ship a real-time LLM system.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    if state != ListingState::Discovered {
        for to in [
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
        ] {
            queries::transition(&pool, &listing_id, to, None)
                .await
                .unwrap();
            if to == state {
                break;
            }
        }
    }
    drop(pool);
    listing_id
}

async fn seed_rendered_application(root: &Path, source: &str) -> (String, String) {
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();

    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: source.into(),
            external_id: "run-rendered-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/run-rendered-1".into(),
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
            profile_hash: "sha256:run-rendered".into(),
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
        ("resume_md", "/tmp/run-it/resume.md", 1024_i64),
        ("resume_docx", "/tmp/run-it/resume.docx", 2048_i64),
        ("cover_md", "/tmp/run-it/cover.md", 512_i64),
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

fn has_pandoc_and_pdf_engine() -> bool {
    which::which("pandoc").is_ok()
        && (which::which("xelatex").is_ok() || which::which("weasyprint").is_ok())
}

#[tokio::test]
async fn run_pipeline_shortlists_tailors_and_optionally_renders_applies() {
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

    seed_listing(root, "greenhouse", ListingState::Discovered).await;

    let report = pipeline::run_pipeline(root, &cfg, false)
        .await
        .expect("run_pipeline should complete");

    assert_eq!(
        report.shortlisted, 1,
        "match stage should shortlist one listing"
    );
    assert_eq!(
        report.tailored, 1,
        "tailor stage should tailor the shortlisted listing"
    );

    if !has_pandoc_and_pdf_engine() {
        eprintln!("skipping render/apply assertions: pandoc or PDF engine not on PATH");
        return;
    }

    assert_eq!(
        report.rendered, 1,
        "render stage should render the tailored application"
    );
    assert_eq!(
        report.apply.dry_run, 1,
        "dry-run run_pipeline must produce exactly one dry-run application"
    );
    assert!(
        report.failures.is_empty(),
        "unexpected failures: {:#?}",
        report.failures
    );
}

#[tokio::test]
async fn run_pipeline_dry_run_applies_rendered_application_without_state_change() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);

    let mut cfg = CoreConfig::load(root).unwrap();
    cfg.submit.per_source.insert(
        "greenhouse".into(),
        SubmitSource {
            enabled: true,
            ..Default::default()
        },
    );

    let (_listing_id, application_id) = seed_rendered_application(root, "greenhouse").await;

    let report = pipeline::run_pipeline(root, &cfg, false)
        .await
        .expect("run_pipeline should complete");

    assert_eq!(report.apply.dry_run, 1);
    assert_eq!(report.apply.submitted, 0);
    assert_eq!(report.apply.failed, 0);

    // Dry-run must not transition the application out of rendered.
    let pool = pool_from_path(&root.join("data/careerai.sqlite"))
        .await
        .unwrap();
    let app = queries::find_application_by_id(&pool, &application_id)
        .await
        .unwrap();
    assert_eq!(app.state, "rendered");
}

#[tokio::test]
async fn run_pipeline_linkedin_interactive_only_drafts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);

    let mut cfg = CoreConfig::load(root).unwrap();
    cfg.submit.auto_submit = true;
    cfg.submit.per_source.insert(
        "linkedin".into(),
        SubmitSource {
            enabled: true,
            ..Default::default()
        },
    );

    let (_listing_id, application_id) = seed_rendered_application(root, "linkedin").await;

    let report = pipeline::run_pipeline(root, &cfg, true)
        .await
        .expect("run_pipeline should complete");

    assert_eq!(
        report.apply.drafted, 1,
        "LinkedIn must be drafted, not submitted"
    );
    assert_eq!(report.apply.submitted, 0);
    assert_eq!(report.apply.failed, 0);

    let pool = pool_from_path(&root.join("data/careerai.sqlite"))
        .await
        .unwrap();
    let app = queries::find_application_by_id(&pool, &application_id)
        .await
        .unwrap();
    assert_eq!(app.state, "drafted");
}

#[tokio::test]
async fn run_pipeline_honors_per_source_gate_by_skipping_disabled_source() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);

    // Explicitly disable greenhouse so the per-source gate is exercised
    // (the embedded default enables it).
    let mut cfg = CoreConfig::load(root).unwrap();
    cfg.submit.per_source.insert(
        "greenhouse".into(),
        SubmitSource {
            enabled: false,
            ..Default::default()
        },
    );

    let (_listing_id, _application_id) = seed_rendered_application(root, "greenhouse").await;

    let report = pipeline::run_pipeline(root, &cfg, false)
        .await
        .expect("run_pipeline should complete");

    assert_eq!(report.apply.skipped, 1);
    assert_eq!(report.apply.submitted, 0);
    assert_eq!(report.apply.dry_run, 0);
    assert_eq!(report.apply.failed, 0);
}
