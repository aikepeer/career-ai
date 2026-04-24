//! Integration coverage for the three ATS HTTP submitters:
//!   - `prepare()` produces the expected URL shape + payload envelope
//!   - `submit()` short-circuits with `SourceDisabled` until the
//!     provider-API-key path is wired (tracked as follow-up work)
//!
//! wiremock is used purely to give each submitter a base URL that
//! definitely does not hit the real internet — it also asserts, via the
//! absence of any matched request, that the live path never actually
//! POSTs during these tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use wiremock::MockServer;

use careerai_db::models::{Application, Artifact, Listing};
use careerai_profile::Profile;
use careerai_submit::{
    AshbySubmitter, GreenhouseSubmitter, LeverSubmitter, SubmitContext, SubmitError, Submitter,
};

fn listing(source: &str, external_id: &str, company: &str) -> Listing {
    let now = Utc::now();
    Listing {
        id: format!("listing-{external_id}"),
        source: source.into(),
        external_id: external_id.into(),
        title: "Engineer".into(),
        company: company.into(),
        location: Some("Remote".into()),
        url: format!("https://example.com/{source}/{external_id}"),
        description: "role".into(),
        raw_json: None,
        state: "rendered".into(),
        score: Some(0.9),
        created_at: now,
        updated_at: now,
    }
}

fn application(listing_id: &str) -> Application {
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

fn profile() -> Profile {
    let mut p = Profile::default();
    p.personal.name = "Alice Applicant".into();
    p.personal.email = "alice@example.com".into();
    p.personal.phone = "555-0100".into();
    p
}

fn artifacts() -> Vec<Artifact> {
    let now = Utc::now();
    vec![
        Artifact {
            id: 1,
            application_id: "app-1".into(),
            kind: "resume_docx".into(),
            path: "/tmp/app-1/resume.docx".into(),
            bytes: 12_345,
            created_at: now,
        },
        Artifact {
            id: 2,
            application_id: "app-1".into(),
            kind: "cover_docx".into(),
            path: "/tmp/app-1/cover.docx".into(),
            bytes: 4_567,
            created_at: now,
        },
    ]
}

#[tokio::test]
async fn greenhouse_prepare_and_stub_submit() {
    let server = MockServer::start().await;
    let listing = listing("greenhouse", "job-42", "Acme Robotics");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts();
    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &arts,
        cover_letter_text: "Dear hiring team,",
    };

    let submitter = GreenhouseSubmitter::new().with_base_url(server.uri());

    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "greenhouse");
    assert_eq!(would.method, "POST");
    // Base URL + path shape.
    assert!(would.url.starts_with(&server.uri()));
    assert!(
        would.url.contains("/v1/boards/acmerobotics/jobs/job-42"),
        "unexpected url: {}",
        would.url
    );
    assert!(would.body_preview.contains("Alice Applicant"));
    assert!(would.body_preview.contains("Acme Robotics"));
    assert_eq!(would.artifact_kinds, vec!["resume_docx", "cover_docx"]);

    let err = submitter.submit(&ctx).await.unwrap_err();
    match err {
        SubmitError::SourceDisabled(msg) => {
            assert!(msg.contains("greenhouse"), "msg: {msg}");
            assert!(msg.contains("requires"), "msg: {msg}");
        }
        other => panic!("expected SourceDisabled, got {other:?}"),
    }

    // No mocks were registered on `server`; if submit() had actually
    // POSTed, wiremock would surface an unmatched-request error here.
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn lever_prepare_and_stub_submit() {
    let server = MockServer::start().await;
    let listing = listing("lever", "posting-7", "Globex Corp");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts();
    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &arts,
        cover_letter_text: "hi",
    };

    let submitter = LeverSubmitter::new().with_base_url(server.uri());

    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "lever");
    assert!(
        would.url.contains("/v0/postings/globexcorp/posting-7"),
        "unexpected url: {}",
        would.url
    );

    let err = submitter.submit(&ctx).await.unwrap_err();
    match err {
        SubmitError::SourceDisabled(msg) => {
            assert!(msg.contains("lever"), "msg: {msg}");
            assert!(msg.contains("requires"), "msg: {msg}");
        }
        other => panic!("expected SourceDisabled, got {other:?}"),
    }

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn ashby_prepare_and_stub_submit() {
    let server = MockServer::start().await;
    let listing = listing("ashby", "pos-9", "Initech");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts();
    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &arts,
        cover_letter_text: "hi",
    };

    let submitter = AshbySubmitter::new().with_base_url(server.uri());

    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "ashby");
    assert!(
        would.url.contains("/applicationForm.submit"),
        "unexpected url: {}",
        would.url
    );

    let err = submitter.submit(&ctx).await.unwrap_err();
    match err {
        SubmitError::SourceDisabled(msg) => {
            assert!(msg.contains("ashby"), "msg: {msg}");
            assert!(msg.contains("requires"), "msg: {msg}");
        }
        other => panic!("expected SourceDisabled, got {other:?}"),
    }

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn body_preview_is_bounded() {
    let listing = listing("greenhouse", "big", "Acme");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts();
    let big = "x".repeat(10_000);
    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &arts,
        cover_letter_text: &big,
    };

    let submitter = GreenhouseSubmitter::new();
    let would = submitter.prepare(&ctx).unwrap();
    // 1024-byte cap on body_preview.
    assert!(
        would.body_preview.len() <= 1024,
        "body_preview was {} bytes",
        would.body_preview.len()
    );
}
