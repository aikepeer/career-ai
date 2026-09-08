//! F02: Verify that `match_all` persists structured match reasons to the
//! `match_reasons` table for shortlisted and below-threshold listings, so
//! the dashboard can always explain *why* a listing got its score or was
//! rejected.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use careerai_core::config::CoreConfig;
use careerai_db::models::NewListing;
use careerai_db::queries;
use careerai_pipeline as pipeline;

fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(
        root.join("profile").join("profile.yaml"),
        "personal:\n  name: \"Test User\"\n  email: \"test@example.com\"\n  phone: \"\"\nsummary: \"Engineer who knows Rust and Python\"\nskills:\n  languages:\n    - \"Rust\"\n    - \"Python\"\n",
    )
    .unwrap();
}

fn disable_all_sources(cfg: &mut CoreConfig) {
    cfg.sources.greenhouse.companies.clear();
    cfg.sources.lever.companies.clear();
    cfg.sources.remotive.enabled = false;
    cfg.sources.remoteok.enabled = false;
    cfg.sources.naukri.enabled = false;
}

async fn seed_listing(
    pool: &sqlx::SqlitePool,
    external_id: &str,
    title: &str,
    company: &str,
    desc: &str,
) -> String {
    let new = NewListing {
        source: "greenhouse".into(),
        external_id: external_id.into(),
        title: title.into(),
        company: company.into(),
        location: Some("Remote".into()),
        url: format!("https://example.com/{external_id}"),
        description: desc.into(),
        raw_json: None,
    };
    let (id, _) = queries::insert_or_ignore(pool, &new).await.unwrap();
    id
}

#[tokio::test]
async fn match_all_persists_reasons_for_shortlisted_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);

    let mut cfg = CoreConfig::load(root).unwrap();
    disable_all_sources(&mut cfg);
    cfg.matching.score_threshold = 0.0; // shortlist everything

    let pool = pipeline::open_pool(root).await.unwrap();
    let listing_id = seed_listing(
        &pool,
        "rust-1",
        "Rust Engineer",
        "Acme Corp",
        "Build distributed systems in Rust with tokio and async I/O. \
         Work on LLM applications and embedded platforms.",
    )
    .await;
    drop(pool);

    let report = pipeline::match_all(root, &cfg, false).await.unwrap();
    assert_eq!(report.shortlisted, 1, "one listing should be shortlisted");

    let pool = pipeline::open_pool(root).await.unwrap();
    let reasons = queries::fetch_match_reasons(&pool, &listing_id)
        .await
        .unwrap()
        .expect("match reasons should be persisted for a shortlisted listing");

    assert!(
        reasons.filter_reason.is_none(),
        "shortlisted listing should have no filter reason, got {:?}",
        reasons.filter_reason
    );
    assert!(
        reasons.score > 0.0,
        "score should be positive, got {}",
        reasons.score
    );
    assert!(
        reasons.matched_keywords.contains("rust"),
        "matched_keywords should contain 'rust', got: {}",
        reasons.matched_keywords
    );
    assert!(
        reasons.legitimacy_tier.is_some(),
        "legitimacy_tier should be set"
    );
}

#[tokio::test]
async fn match_all_persists_reasons_for_below_threshold_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    scaffold_project(root);

    let mut cfg = CoreConfig::load(root).unwrap();
    disable_all_sources(&mut cfg);
    cfg.matching.score_threshold = 0.99; // nothing can clear this

    let pool = pipeline::open_pool(root).await.unwrap();
    let listing_id = seed_listing(
        &pool,
        "low-score-1",
        "Java Engineer",
        "Beta Corp",
        "Build enterprise Java applications with Spring Boot and Hibernate. \
         No Rust, no embedded, no LLM work here.",
    )
    .await;
    drop(pool);

    let report = pipeline::match_all(root, &cfg, false).await.unwrap();
    assert_eq!(report.also_filtered, 1, "one listing should be below threshold");

    let pool = pipeline::open_pool(root).await.unwrap();
    let reasons = queries::fetch_match_reasons(&pool, &listing_id)
        .await
        .unwrap()
        .expect("match reasons should be persisted for below-threshold listings");

    assert!(
        reasons.filter_reason.is_some(),
        "below-threshold listing must have a filter reason"
    );
    assert!(
        reasons
            .filter_reason
            .as_ref()
            .unwrap()
            .contains("below threshold"),
        "filter reason should mention 'below threshold', got: {:?}",
        reasons.filter_reason
    );
}
