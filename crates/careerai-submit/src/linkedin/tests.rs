#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use chrono::Utc;

use careerai_db::models::{Application, Artifact, Listing};
use careerai_profile::schema::Profile;

use super::config::LinkedinConfig;
use super::submitter::LinkedinSubmitter;
use crate::base::{SubmitContext, Submitter};
use crate::rate_limiter::RateLimiter;

fn fixture_listing() -> Listing {
    let now = Utc::now();
    Listing {
        id: "l-1".into(),
        source: "linkedin".into(),
        external_id: "4123456789".into(),
        title: "Senior ML Engineer".into(),
        company: "Acme".into(),
        location: Some("Remote, India".into()),
        url: "https://www.linkedin.com/jobs/view/4123456789".into(),
        description: "...".into(),
        raw_json: None,
        state: "rendered".into(),
        score: Some(0.9),
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
        profile_hash: "sha256:x".into(),
        prompt_version: "tailor.v1".into(),
        llm_model: "claude-3-5-sonnet".into(),
        created_at: now,
        updated_at: now,
    }
}

fn fixture_profile() -> Profile {
    let mut p = Profile::default();
    p.personal.name = "Ada Applicant".into();
    p.personal.email = "ada@example.com".into();
    p
}

#[test]
fn prepare_builds_listing_url_from_external_id() {
    let listing = fixture_listing();
    let application = fixture_application(&listing.id);
    let profile = fixture_profile();
    let artifacts: Vec<Artifact> = Vec::new();
    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &artifacts,
        cover_letter_text: "test",
    };
    let sub = LinkedinSubmitter::new(LinkedinConfig::default(), Arc::new(RateLimiter::new()));
    let would = sub.prepare(&ctx).unwrap();
    assert_eq!(would.source, "linkedin");
    assert_eq!(would.method, "BROWSER");
    assert!(
        would.url.contains("/jobs/view/4123456789"),
        "url={}",
        would.url
    );
    assert!(
        !would.body_preview.contains("ada@example.com"),
        "leaked email: {}",
        would.body_preview
    );
    assert!(
        !would.body_preview.contains("Ada Applicant"),
        "leaked name: {}",
        would.body_preview
    );
    assert!(
        would.body_preview.contains("Senior ML Engineer"),
        "missing title: {}",
        would.body_preview
    );
    assert!(
        would.body_preview.contains("Acme"),
        "missing company: {}",
        would.body_preview
    );
}

#[test]
fn prepare_sanitizes_external_id_against_path_traversal() {
    let mut listing = fixture_listing();
    listing.external_id = "../../settings?tab=account".into();
    let application = fixture_application(&listing.id);
    let profile = fixture_profile();
    let artifacts: Vec<Artifact> = Vec::new();
    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &artifacts,
        cover_letter_text: "x",
    };
    let sub = LinkedinSubmitter::new(LinkedinConfig::default(), Arc::new(RateLimiter::new()));
    let would = sub.prepare(&ctx).unwrap();
    assert!(!would.url.contains(".."), "url leaked '..': {}", would.url);
    assert!(!would.url.contains('?'), "url leaked '?': {}", would.url);
    assert!(!would.url.contains('='), "url leaked '=': {}", would.url);
    assert!(
        would.url.starts_with("https://www.linkedin.com/jobs/view/"),
        "url escaped origin: {}",
        would.url
    );
}

#[test]
fn debug_omits_full_config() {
    let sub = LinkedinSubmitter::new(LinkedinConfig::default(), Arc::new(RateLimiter::new()));
    let s = format!("{sub:?}");
    assert!(s.contains("LinkedinSubmitter"), "got: {s}");
    assert!(s.contains("max_per_day"), "got: {s}");
    assert!(!s.contains("screenshots_dir"), "leaked path: {s}");
}
