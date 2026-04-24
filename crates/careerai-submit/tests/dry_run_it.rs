//! Dry-run integration: wrapping any real submitter in `DryRunSubmitter`
//! must return a deterministic pseudo-id without touching the network.
//!
//! We don't need wiremock here — the whole point is that no HTTP client
//! is invoked on the dry-run path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;

use careerai_db::models::{Application, Artifact, Listing};
use careerai_profile::Profile;
use careerai_submit::{DryRunSubmitter, GreenhouseSubmitter, SubmitContext, Submitter};

fn fixture_listing() -> Listing {
    let now = Utc::now();
    Listing {
        id: "listing-1".into(),
        source: "greenhouse".into(),
        external_id: "job-42".into(),
        title: "Senior ML Engineer".into(),
        company: "Acme Robotics".into(),
        location: Some("Remote".into()),
        url: "https://boards.greenhouse.io/acme/jobs/42".into(),
        description: "Build embedded LLM perception systems.".into(),
        raw_json: None,
        state: "rendered".into(),
        score: Some(0.88),
        created_at: now,
        updated_at: now,
    }
}

fn fixture_application(listing_id: &str) -> Application {
    let now = Utc::now();
    Application {
        id: "app-1".into(),
        listing_id: listing_id.into(),
        state: "rendered".into(),
        profile_hash: "sha256:abc".into(),
        prompt_version: "tailor.v1".into(),
        llm_model: "claude-3-5-sonnet-20241022".into(),
        created_at: now,
        updated_at: now,
    }
}

fn fixture_profile() -> Profile {
    let mut p = Profile::default();
    p.personal.name = "Alice Applicant".into();
    p.personal.email = "alice@example.com".into();
    p
}

#[tokio::test]
async fn dry_run_returns_deterministic_pseudo_id_without_network() {
    let listing = fixture_listing();
    let application = fixture_application(&listing.id);
    let profile = fixture_profile();
    let artifacts: Vec<Artifact> = Vec::new();

    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &artifacts,
        cover_letter_text: "test body",
    };

    // Base URL is localhost:0 — if the dry-run path ever issued a real
    // HTTP request, it would fail with ConnectionRefused. Returning
    // `Ok("dry-run:greenhouse")` proves no network write happened.
    let submitter =
        DryRunSubmitter::new(GreenhouseSubmitter::new().with_base_url("http://127.0.0.1:0"));

    let id = submitter.submit(&ctx).await.expect("dry-run never errors");
    assert_eq!(id, "dry-run:greenhouse");
}

#[tokio::test]
async fn dry_run_prepare_includes_body_preview() {
    let listing = fixture_listing();
    let application = fixture_application(&listing.id);
    let profile = fixture_profile();
    let artifacts: Vec<Artifact> = Vec::new();

    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &artifacts,
        cover_letter_text: "cover letter body",
    };

    let submitter = DryRunSubmitter::new(GreenhouseSubmitter::new());
    let would = submitter.prepare(&ctx).expect("prepare is pure");
    assert_eq!(would.source, "greenhouse");
    assert_eq!(would.method, "POST");
    assert!(would.body_preview.contains("Alice Applicant"));
    assert!(would.body_preview.contains("cover letter body"));
    assert!(would.url.contains("/jobs/job-42"));
}
