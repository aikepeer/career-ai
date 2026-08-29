//! Live-tailor policy tests. These tests use local CLI stubs only: no network
//! and no real provider credentials are required.

#![cfg(unix)]
#![cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
#![allow(clippy::await_holding_lock)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Mutex;

use careerai_core::config::{BackendChoice, CoreConfig};
use careerai_core::state::ListingState;
use careerai_db::models::NewListing;
use careerai_db::{pool_from_path, queries};
use careerai_pipeline::tailor_one;
use careerai_profile::schema::{Experience, Personal, Profile, Project};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const TAILOR_DIFF: &str = r#"{
    "prompt_version":"tailor.v1",
    "summary":{"op":"keep"},
    "ops":[
        {"path":"experience[0].bullets[0]","op":"keep"},
        {"path":"projects[0].bullets[0]","op":"keep"}
    ],
    "cover_letter":"short"
}"#;

const COVER_LETTER: &str = "Dear Hiring Team,\n\nI am applying.\n\nRegards.";

fn fixture_profile() -> Profile {
    Profile {
        personal: Personal {
            name: "Ada Lovelace".into(),
            ..Default::default()
        },
        experience: vec![Experience {
            title: "Engineer".into(),
            company: "Acme Robotics".into(),
            start: "2022-01".into(),
            end: "present".into(),
            bullets: vec!["Built Rust services.".into()],
            ..Default::default()
        }],
        projects: vec![Project {
            name: "Open project".into(),
            bullets: vec!["Created a useful tool.".into()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

async fn setup() -> (tempfile::TempDir, CoreConfig, String) {
    let root = tempfile::tempdir().unwrap();
    for directory in ["config", "profile", "data"] {
        fs::create_dir_all(root.path().join(directory)).unwrap();
    }
    fs::write(
        root.path().join("profile/profile.yaml"),
        fixture_profile().to_yaml().unwrap(),
    )
    .unwrap();
    let cfg = CoreConfig::load(root.path()).unwrap();
    let pool = pool_from_path(&root.path().join("data/careerai.sqlite"))
        .await
        .unwrap();
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "fixture".into(),
            external_id: "live-policy-1".into(),
            title: "Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/live-policy-1".into(),
            description: "Build reliable Rust services.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    queries::transition(&pool, &listing_id, ListingState::Shortlisted, None)
        .await
        .unwrap();
    drop(pool);
    (root, cfg, listing_id)
}

fn write_fixtures(root: &Path) {
    let directory = root.join("data/cache/llm/fixtures");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("tailor.v1.json"), TAILOR_DIFF).unwrap();
    fs::write(directory.join("tailor.v1+cover_letter.json"), COVER_LETTER).unwrap();
}

fn write_stub(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn enable_live() -> Option<String> {
    let previous = std::env::var("CAREERAI_LLM_LIVE").ok();
    std::env::set_var("CAREERAI_LLM_LIVE", "1");
    previous
}

fn restore_live(previous: Option<String>) {
    if let Some(value) = previous {
        std::env::set_var("CAREERAI_LLM_LIVE", value);
    } else {
        std::env::remove_var("CAREERAI_LLM_LIVE");
    }
}

#[tokio::test]
async fn forced_backend_failure_does_not_use_fixtures() -> TestResult {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (root, mut cfg, listing_id) = setup().await;
    write_fixtures(root.path());
    cfg.llm.backend = BackendChoice::Goose;
    let previous = enable_live();
    let previous_path = std::env::var("PATH").ok();
    std::env::set_var("PATH", root.path().join("empty-bin"));
    let result = tailor_one(root.path(), &cfg, &listing_id).await;
    restore_live(previous);
    if let Some(path) = previous_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }

    assert!(result.is_err(), "forced Goose failure must be surfaced");
    let pool = pool_from_path(&root.path().join("data/careerai.sqlite")).await?;
    let listing = queries::find_by_id(&pool, &listing_id).await?;
    assert_eq!(listing.state, ListingState::Shortlisted.as_str());
    Ok(())
}

#[tokio::test]
async fn malformed_live_output_does_not_fallback_or_cache() -> TestResult {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (root, mut cfg, listing_id) = setup().await;
    write_fixtures(root.path());
    let stub = root.path().join("malformed-cli");
    write_stub(&stub, "#!/bin/sh\ncat >/dev/null\nprintf '%s' 'not-json'\n");
    cfg.llm.backend = BackendChoice::CustomCli(stub.display().to_string());
    let previous = enable_live();
    let result = tailor_one(root.path(), &cfg, &listing_id).await;
    restore_live(previous);

    let error = result.expect_err("malformed live output must fail");
    assert!(
        format!("{error:#}").contains("malformed JSON"),
        "error={error:#}"
    );
    let cache_root = root.path().join("data/cache/llm");
    let response_files = fs::read_dir(&cache_root)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .count();
    assert_eq!(response_files, 0, "failed live output must not be cached");
    Ok(())
}

#[tokio::test]
async fn cover_letter_failure_is_not_replaced_by_fixture() -> TestResult {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (root, mut cfg, listing_id) = setup().await;
    write_fixtures(root.path());
    let escaped_diff = serde_json::to_string(TAILOR_DIFF)?;
    let stub = root.path().join("cover-fails-cli");
    let script = format!(
        r#"#!/bin/sh
input=$(cat)
case "$input" in
  *"Draft a cover letter"*) printf '%s' 'not-json' ;;
  *) printf '%s' '{{"is_error":false,"result":{escaped_diff}}}' ;;
esac
"#,
    );
    write_stub(&stub, &script);
    cfg.llm.backend = BackendChoice::CustomCli(stub.display().to_string());
    let previous = enable_live();
    let result = tailor_one(root.path(), &cfg, &listing_id).await;
    restore_live(previous);

    let error = result.expect_err("cover-letter live failure must fail the stage");
    assert!(
        format!("{error:#}").contains("malformed JSON"),
        "error={error:#}"
    );
    let pool = pool_from_path(&root.path().join("data/careerai.sqlite")).await?;
    let listing = queries::find_by_id(&pool, &listing_id).await?;
    assert_eq!(listing.state, ListingState::Shortlisted.as_str());
    let cache_root = root.path().join("data/cache/llm");
    let response_files = fs::read_dir(&cache_root)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .count();
    assert_eq!(
        response_files, 1,
        "only the validated tailor response may be cached"
    );
    Ok(())
}
