//! End-to-end integration test for `pipeline::tailor_one` + `pipeline::render_one`.
//!
//! Uses `MockLlm` fixtures (no network) and an in-repo SQLite file under a
//! tempdir. The `render_one` half is gated on `pandoc` being on PATH — when
//! pandoc is missing we still assert the `tailor_one` half end-to-end and
//! log-skip the render half.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use std::fs;
use std::path::Path;

use careerai_core::config::CoreConfig;
use careerai_db::models::NewListing;
use careerai_db::{pool_from_path, queries};
use careerai_pipeline as pipeline;
use careerai_profile::schema::{Experience, Personal, Profile, Project, Skills};
use sqlx::Row;

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
        education: vec![careerai_profile::schema::Education {
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

/// Scaffold the minimum directory tree the pipeline expects under `root`.
fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    // Write the profile YAML.
    let profile_yaml = fixture_profile().to_yaml().unwrap();
    fs::write(root.join("profile").join("profile.yaml"), profile_yaml).unwrap();
    // Write an explicit default.yaml so the loaded `CoreConfig` picks up our
    // artifacts_dir (resolved relative to `root` by `render_one`).
    fs::write(
        root.join("config").join("default.yaml"),
        // A trimmed config — the embedded defaults in CoreConfig fill the rest.
        "render:\n  artifacts_dir: \"artifacts\"\n  pandoc_bin: null\n  pdf_engine: \"weasyprint\"\n  timeout_seconds: 60\n  keep_intermediate_markdown: true\n",
    )
    .unwrap();
}

/// Write the MockLlm fixtures into `<root>/data/cache/llm/fixtures/` — the
/// default location `pipeline::fixtures_dir` falls back to when the
/// `CAREERAI_LLM_FIXTURES_DIR` env var is unset. We deliberately don't set
/// the env var here because `std::env::set_var` is `unsafe` in modern Rust
/// and the workspace forbids `unsafe_code`; using the default path keeps
/// the test zero-env.
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

#[tokio::test]
async fn tailor_then_render_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);
    write_fixtures(root);

    // Load config (defaults + our minimal default.yaml override).
    let mut cfg = CoreConfig::load(root).expect("load cfg");
    // Route the LLM cache under the tempdir so the test never leaks writes
    // into the real repo's `data/cache/llm/` (the default is relative).
    cfg.llm.cache_dir = root
        .join("data")
        .join("cache")
        .join("llm")
        .to_string_lossy()
        .into_owned();

    // Bring up the DB + seed a shortlisted listing.
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.expect("open pool");
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "fixture".into(),
            external_id: "it-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/it-1".into(),
            description: "Build and ship a real-time LLM system.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    queries::transition(
        &pool,
        &listing_id,
        careerai_core::state::ListingState::Shortlisted,
        None,
    )
    .await
    .unwrap();
    // Close our handle so the pipeline opens its own pool cleanly.
    drop(pool);

    // --- Tailor half -------------------------------------------------------
    let tailored = pipeline::tailor_one(root, &cfg, &listing_id)
        .await
        .expect("tailor_one ok");
    assert!(!tailored.application_id.is_empty(), "no application_id");
    assert_eq!(tailored.listing_title, "Senior Rust Engineer");
    assert_eq!(tailored.company, "Beta Corp");

    // Open a fresh pool to verify post-conditions.
    let pool = pool_from_path(&db_path).await.unwrap();

    // application exists + is in 'tailored'.
    let app = queries::find_application_by_id(&pool, &tailored.application_id)
        .await
        .unwrap();
    assert_eq!(app.state, "tailored");
    assert_eq!(app.listing_id, listing_id);

    // listing is now 'tailored'.
    let listing = queries::find_by_id(&pool, &listing_id).await.unwrap();
    assert_eq!(listing.state, "tailored");

    // payload row exists.
    let payload = queries::find_payload_by_application_id(&pool, &tailored.application_id)
        .await
        .unwrap();
    assert!(
        !payload.resume_view_json.is_empty(),
        "payload resume_view_json empty"
    );
    assert!(
        !payload.cover_letter_text.is_empty(),
        "payload cover_letter_text empty"
    );

    // --- Render half -------------------------------------------------------
    if which::which("pandoc").is_err() {
        eprintln!("skipping render_one assertions: pandoc not on PATH");
        return;
    }

    // PDF engine is required for the PDF step. If none present, pandoc's PDF
    // pass will fail; we detect + skip gracefully.
    let has_pdf_engine = which::which("xelatex").is_ok() || which::which("weasyprint").is_ok();
    if !has_pdf_engine {
        eprintln!("skipping render_one assertions: no PDF engine (xelatex/weasyprint) on PATH");
        return;
    }

    // Resolve PDF engine for the render config so the test works regardless
    // of the host's default.
    if which::which("xelatex").is_ok() {
        cfg.render.pdf_engine = "xelatex".into();
    } else {
        cfg.render.pdf_engine = "weasyprint".into();
    }

    let rendered = pipeline::render_one(root, &cfg, &tailored.application_id)
        .await
        .expect("render_one ok");
    assert_eq!(rendered.application_id, tailored.application_id);

    // Files on disk in expected locations.
    let app_dir = root.join("artifacts").join(&tailored.application_id);
    for f in [
        "resume.md",
        "resume.docx",
        "resume.pdf",
        "cover_letter.md",
        "cover_letter.docx",
    ] {
        let p = app_dir.join(f);
        assert!(p.is_file(), "missing artifact: {}", p.display());
    }
    let pdf_size = fs::metadata(app_dir.join("resume.pdf")).unwrap().len();
    assert!(
        pdf_size >= 100,
        "resume.pdf is suspiciously small: {pdf_size} bytes"
    );

    // DOCX content sanity check: extract and look for name + bullet text.
    let docx_path = app_dir.join("resume.docx");
    let docx_bytes = fs::read(&docx_path).unwrap();
    let text = extract_docx_text(&docx_bytes);
    assert!(text.contains("Ada Lovelace"), "name missing from docx");
    assert!(
        text.contains("Rust LLM pipeline") || text.contains("embedded perception"),
        "no original bullet substring found in docx; extracted text: {text}"
    );

    // 5 artifact rows in the DB.
    let artifact_rows = queries::list_artifacts(&pool, &tailored.application_id)
        .await
        .unwrap();
    assert_eq!(
        artifact_rows.len(),
        5,
        "expected 5 artifact rows, got {}",
        artifact_rows.len()
    );

    // listing state + application state == 'rendered'.
    let listing = queries::find_by_id(&pool, &listing_id).await.unwrap();
    assert_eq!(listing.state, "rendered");
    let app = queries::find_application_by_id(&pool, &tailored.application_id)
        .await
        .unwrap();
    assert_eq!(app.state, "rendered");

    // Sanity: bytes map is non-trivial for every artifact.
    let row = sqlx::query("SELECT COUNT(*) FROM artifacts WHERE bytes > 0 AND application_id = ?")
        .bind(&tailored.application_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let non_zero: i64 = row.get(0);
    assert_eq!(non_zero, 5, "some artifact rows have zero bytes");
}

/// Extract the flattened text content of a DOCX file via `docx-rs`. We
/// serialize the parsed document to JSON and substring-match against it —
/// sufficient for the `contains(name)` / `contains(bullet)` assertions
/// in this test without hand-rolling a zip + XML reader.
fn extract_docx_text(bytes: &[u8]) -> String {
    let docx = docx_rs::read_docx(bytes).expect("parse docx");
    docx.json()
}
