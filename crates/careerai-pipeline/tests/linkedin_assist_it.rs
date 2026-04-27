//! Integration test: LinkedIn applications with interactive_only=true
//! transition to Drafted instead of attempting browser submission.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::{models::*, pool_from_path, queries};
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
    // Minimal config override — loader merges with embedded defaults.
    fs::write(
        root.join("config").join("default.yaml"),
        "render:\n  artifacts_dir: \"artifacts\"\n",
    )
    .unwrap();
}

async fn seed_rendered_linkedin_application(root: &Path) -> (String, String) {
    let db_path = root.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();

    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "linkedin".into(),
            external_id: "linkedin-assist-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://linkedin.com/jobs/view/123".into(),
            description: "Build LLM systems in Rust.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    // Walk the listing through to Rendered so the submit invariant is satisfied.
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
            profile_hash: "sha256:linkedin-assist".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "mock".into(),
        },
    )
    .await
    .unwrap();
    queries::set_application_state(&pool, &app.id, "rendered")
        .await
        .unwrap();

    drop(pool);
    (listing_id, app.id)
}

#[tokio::test]
async fn linkedin_apply_with_interactive_only_transitions_to_drafted() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());
    let (_listing_id, application_id) = seed_rendered_linkedin_application(tmp.path()).await;

    let mut cfg = CoreConfig::load(tmp.path()).unwrap();
    cfg.submit.auto_submit = true;
    cfg.submit.per_source.insert(
        "linkedin".into(),
        careerai_core::config::SubmitSource { enabled: true },
    );
    // Default is true, but be explicit about the contract under test.
    cfg.submit.linkedin.interactive_only = true;

    let outcome = pipeline::apply_one(tmp.path(), &cfg, &application_id, None)
        .await
        .expect("apply_one should succeed and transition to drafted");

    // The DB row must reflect Drafted.
    let pool = pool_from_path(&tmp.path().join("data/careerai.sqlite"))
        .await
        .unwrap();
    let app = queries::find_application_by_id(&pool, &application_id)
        .await
        .unwrap();
    assert_eq!(app.state, "drafted", "expected Drafted, got {}", app.state);

    // Outcome must be Drafted (state changed, no network submission).
    // Submitted/DryRun would both be wrong: Submitted = network call
    // happened (daemon never auto-submits LinkedIn); DryRun = no state
    // change (we DID write Drafted to the DB).
    match &outcome.outcome {
        careerai_submit::SubmitOutcome::Drafted { .. } => {}
        other => panic!("interactive_only=true must yield Drafted, got {other:?}"),
    }
}

#[tokio::test]
async fn list_drafted_linkedin_round_trips_via_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    // Seed a drafted linkedin application via the DB helpers.
    let (_listing_id, application_id) = seed_rendered_linkedin_application(tmp.path()).await;

    // Set it to drafted (simulating what apply_one does with interactive_only).
    let db_path = tmp.path().join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();
    queries::set_application_state(&pool, &application_id, "drafted")
        .await
        .unwrap();

    // Seed a second listing from greenhouse in drafted state — should not appear.
    let (gh_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "gh-pipeline-wrapper-1".into(),
            title: "Staff Engineer".into(),
            company: "Gamma Corp".into(),
            location: Some("Remote".into()),
            url: "https://greenhouse.io/jobs/1".into(),
            description: "Build distributed systems.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    let gh_app = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: gh_id,
            profile_hash: "sha256:gh-wrapper".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "mock".into(),
        },
    )
    .await
    .unwrap();
    queries::set_application_state(&pool, &gh_app.id, "drafted")
        .await
        .unwrap();
    drop(pool);

    let drafts = pipeline::list_drafted_linkedin(tmp.path(), 100)
        .await
        .unwrap();
    assert_eq!(drafts.len(), 1, "only the linkedin draft must appear");
    assert_eq!(drafts[0].id, application_id);
    assert_eq!(drafts[0].state, "drafted");
}

#[tokio::test]
async fn confirm_linkedin_submit_rejects_non_drafted_state() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    // seed_rendered_linkedin_application leaves the application in `rendered`.
    let (_listing_id, application_id) = seed_rendered_linkedin_application(tmp.path()).await;

    let cfg = CoreConfig::load(tmp.path()).unwrap();

    // confirm_linkedin_submit must reject applications that are not in Drafted.
    let err = pipeline::confirm_linkedin_submit(tmp.path(), &cfg, &application_id)
        .await
        .expect_err("should fail because application is in rendered, not drafted");

    let msg = format!("{err}");
    assert!(
        msg.contains("expected 'drafted'"),
        "error message should mention expected state; got: {msg}",
    );
}

/// Positive-path test: a Drafted LinkedIn application must NOT be
/// rejected by `submit_application`'s state guard. The earlier guard
/// allowed only `Rendered | Prepared`, so `confirm_linkedin_submit`
/// always returned `BadState { state: "drafted" }` — see PR #11 review
/// (P0). The test calls `submit_application` directly with a Drafted row
/// and asserts the error (if any) is anything other than `BadState`. Any
/// downstream failure (e.g. missing payload row) is unrelated to the
/// state guard and outside the P0's scope.
#[tokio::test]
async fn submit_application_accepts_drafted_state() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let (_listing_id, application_id) = seed_rendered_linkedin_application(tmp.path()).await;

    let db_path = tmp.path().join("data/careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();
    queries::set_application_state(&pool, &application_id, "drafted")
        .await
        .unwrap();

    let mut cfg = CoreConfig::load(tmp.path()).unwrap();
    cfg.submit.auto_submit = false;
    cfg.submit.per_source.insert(
        "linkedin".into(),
        careerai_core::config::SubmitSource { enabled: true },
    );

    let result =
        careerai_submit::submit_application(&pool, &cfg.submit, tmp.path(), &application_id).await;

    // The state guard must not reject Drafted. Any other error variant
    // (Db, Io, etc.) means the guard let us through, which is the P0
    // contract under test.
    if let Err(careerai_submit::SubmitError::BadState { state }) = &result {
        panic!("submit_application rejected Drafted state with BadState({state}); P0 regression");
    }
}

/// Regression test for the double-submit race: two `careerai review`
/// processes both observe `state = drafted` simultaneously. With the
/// atomic claim in `confirm_linkedin_submit`, exactly one wins and the
/// other gets a clear error before any browser launch.
#[tokio::test]
async fn confirm_linkedin_submit_atomic_claim_prevents_double_submit() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let (_listing_id, application_id) = seed_rendered_linkedin_application(tmp.path()).await;

    // Move the application into Drafted (post-daemon-draft state).
    let db_path = tmp.path().join("data/careerai.sqlite");
    let pool = pool_from_path(&db_path).await.unwrap();
    queries::set_application_state(&pool, &application_id, "drafted")
        .await
        .unwrap();

    // First claim wins — state transitions Drafted → Rendered.
    let won = queries::claim_drafted_application(&pool, &application_id)
        .await
        .unwrap();
    assert!(won, "first claim must succeed");

    let app_after = queries::find_application_by_id(&pool, &application_id)
        .await
        .unwrap();
    assert_eq!(app_after.state, "rendered", "claim transitions to rendered");

    // Second claim loses — state is no longer Drafted.
    let won_again = queries::claim_drafted_application(&pool, &application_id)
        .await
        .unwrap();
    assert!(!won_again, "second claim on already-rendered row must fail");
}
