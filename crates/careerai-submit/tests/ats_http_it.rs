//! Integration coverage for the ATS HTTP submitters.
//!
//! - `prepare()` produces the expected URL shape + payload envelope.
//! - `submit()` POSTs to a wiremock base URL (never the real internet) and
//!   surfaces the remote id / status.
//!
//! The resume artifact is written to a real temp file so the submitters'
//! resume-reading path is exercised.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use careerai_db::models::{Application, Artifact, Listing};
use careerai_profile::Profile;
use careerai_submit::{
    AshbySubmitter, GreenhouseSubmitter, LeverSubmitter, SmartRecruitersSubmitter, SubmitContext,
    Submitter, TeamtailorSubmitter,
};

fn listing(source: &str, external_id: &str, company: &str) -> Listing {
    let now = Utc::now();
    let url = match source {
        "greenhouse" => format!("https://boards.greenhouse.io/acme/jobs/{external_id}"),
        "lever" => format!("https://jobs.lever.co/globexcorp/{external_id}"),
        "ashby" => format!("https://jobs.ashbyhq.com/initech/{external_id}"),
        other => format!("https://example.com/{other}/{external_id}"),
    };
    Listing {
        id: format!("listing-{external_id}"),
        source: source.into(),
        external_id: external_id.into(),
        title: "Engineer".into(),
        company: company.into(),
        location: Some("Remote".into()),
        url,
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

/// Write a real resume file and return the artifact rows pointing at it.
fn artifacts(dir: &std::path::Path) -> Vec<Artifact> {
    let now = Utc::now();
    let resume_path = dir.join("resume.pdf");
    std::fs::write(&resume_path, b"%PDF-1.4 fake resume").unwrap();
    vec![
        Artifact {
            id: 1,
            application_id: "app-1".into(),
            kind: "resume_pdf".into(),
            path: resume_path.display().to_string(),
            bytes: 12_345,
            created_at: now,
        },
        Artifact {
            id: 2,
            application_id: "app-1".into(),
            kind: "cover_docx".into(),
            path: dir.join("cover.docx").display().to_string(),
            bytes: 4_567,
            created_at: now,
        },
    ]
}

fn ctx<'a>(
    listing: &'a Listing,
    application: &'a Application,
    profile: &'a Profile,
    arts: &'a [Artifact],
) -> SubmitContext<'a> {
    SubmitContext {
        application,
        listing,
        profile,
        artifacts: arts,
        cover_letter_text: "Dear hiring team,",
    }
}

#[tokio::test]
async fn greenhouse_prepare_and_submit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/boards/acme/jobs/job-42"))
        .respond_with(ResponseTemplate::new(200).set_body_string("greenhouse-remote-id"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let listing = listing("greenhouse", "job-42", "Acme Robotics");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts(tmp.path());
    let ctx = ctx(&listing, &application, &profile, &arts);

    let submitter = GreenhouseSubmitter::new().with_base_url(server.uri());

    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "greenhouse");
    assert_eq!(would.method, "POST");
    assert!(
        would.url.contains("/v1/boards/acme/jobs/job-42"),
        "unexpected url: {}",
        would.url
    );
    assert!(would.body_preview.contains("Alice Applicant"));

    let remote_id = submitter.submit(&ctx).await.unwrap();
    assert_eq!(remote_id, "greenhouse-remote-id");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["first_name"], "Alice");
    assert_eq!(body["last_name"], "Applicant");
    assert_eq!(body["email"], "alice@example.com");
    assert!(body["resume"].as_str().is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn lever_prepare_and_submit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/postings/posting-7/apply"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let listing = listing("lever", "posting-7", "Globex Corp");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts(tmp.path());
    let ctx = ctx(&listing, &application, &profile, &arts);

    let submitter = LeverSubmitter::new().with_base_url(server.uri());

    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "lever");
    assert!(
        would.url.contains("/v1/postings/posting-7/apply"),
        "unexpected url: {}",
        would.url
    );

    let remote_id = submitter.submit(&ctx).await.unwrap();
    // Empty response → falls back to a deterministic synthetic id.
    assert_eq!(remote_id, "lever:posting-7");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn ashby_prepare_and_submit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/applicationForm.submit"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ashby-remote-id"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let listing = listing("ashby", "pos-9", "Initech");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts(tmp.path());
    let ctx = ctx(&listing, &application, &profile, &arts);

    let submitter = AshbySubmitter::new().with_base_url(server.uri());

    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "ashby");
    assert!(would.url.contains("/applicationForm.submit"));

    let remote_id = submitter.submit(&ctx).await.unwrap();
    assert_eq!(remote_id, "ashby-remote-id");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn smartrecruiters_prepare_and_submit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/companies/Acme/postings/pos-9"))
        .respond_with(ResponseTemplate::new(200).set_body_string("sr-remote-id"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let mut listing = listing("smartrecruiters", "pos-9", "Acme");
    listing.url = "https://jobs.smartrecruiters.com/Acme/pos-9".to_string();
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts(tmp.path());
    let ctx = ctx(&listing, &application, &profile, &arts);

    let submitter = SmartRecruitersSubmitter::new().with_base_url(server.uri());
    let would = submitter.prepare(&ctx).unwrap();
    assert_eq!(would.source, "smartrecruiters");
    assert!(
        would.url.contains("/v1/companies/Acme/postings/pos-9"),
        "unexpected url: {}",
        would.url
    );

    let remote_id = submitter.submit(&ctx).await.unwrap();
    assert_eq!(remote_id, "sr-remote-id");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn teamtailor_submit_posts_multipart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jobs/job-1/application"))
        .respond_with(ResponseTemplate::new(200).set_body_string("tt-remote-id"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let mut listing = listing("teamtailor", "job-1", "Synmatch AI");
    listing.url = format!("{}/jobs/job-1", server.uri());
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts(tmp.path());
    let ctx = ctx(&listing, &application, &profile, &arts);

    let submitter = TeamtailorSubmitter::new();
    let would = submitter.prepare(&ctx).unwrap();
    assert!(would.url.ends_with("/jobs/job-1/application"));

    let remote_id = submitter.submit(&ctx).await.unwrap();
    assert_eq!(remote_id, "tt-remote-id");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn body_preview_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let listing = listing("greenhouse", "big", "Acme");
    let application = application(&listing.id);
    let profile = profile();
    let arts = artifacts(tmp.path());
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
    assert!(
        would.body_preview.len() <= 1024,
        "body_preview was {} bytes",
        would.body_preview.len()
    );
}
