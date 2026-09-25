//! Integration tests for PR 12: ops, metrics, export format,
//! artifact store, billing adapter, and deletion/export drills.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_hosted::ops::billing_adapter::{
    BillingAdapter, CheckoutSessionRequest, MockBillingAdapter, StripeBillingAdapter,
};
use careerai_hosted::ops::export_format::{ExportBuilder, SCHEMA_VERSION};
use careerai_hosted::ops::metrics::{MetricEvent, MetricsCollector};
use careerai_hosted::ops::object_store::ArtifactStore;

// ── Metrics ─────────────────────────────────────────────────────────

#[test]
fn metrics_tracks_beta_exit_criteria() {
    let m = MetricsCollector::new();

    // Simulate 5 users completing onboarding
    for i in 1..=5 {
        m.record(MetricEvent::OnboardingCompleted, &format!("t{i}"));
    }

    // Simulate 20 match reviews across 5 tenants
    for i in 1..=20 {
        m.record(MetricEvent::MatchReviewed, &format!("t{}", (i % 5) + 1));
    }

    // Simulate 3 programs completed
    for i in 1..=3 {
        m.record(MetricEvent::ProgramCompleted, &format!("t{i}"));
    }

    // Simulate 1 export
    m.record(MetricEvent::ExportCompleted, "t1");

    let snap = m.beta_exit_snapshot();
    assert_eq!(snap.onboarding_completed, 5);
    assert_eq!(snap.match_reviewed_total, 20);
    assert_eq!(snap.program_completed, 3);
    assert_eq!(snap.export_completed_total, 1);
}

#[test]
fn prometheus_exposition_is_valid_format() {
    let m = MetricsCollector::new();
    m.record(MetricEvent::OnboardingCompleted, "t1");
    m.record(MetricEvent::MatchReviewed, "t1");
    m.record_latency("/v1/listings", 42.0);
    m.record_latency("/v1/listings", 88.0);

    let text = m.render_prometheus();
    assert!(text.contains("# TYPE careerai_onboarding_completed counter"));
    assert!(text.contains("careerai_onboarding_completed_total 1"));
    assert!(text.contains("# TYPE careerai_match_reviewed counter"));
    assert!(text.contains("careerai_match_reviewed_total 1"));
    assert!(text.contains("careerai_api_latency_ms"));
}

#[test]
fn metrics_track_action_approvals_and_denials() {
    let m = MetricsCollector::new();
    m.record(MetricEvent::ActionApproved, "t1");
    m.record(MetricEvent::ActionApproved, "t1");
    m.record(MetricEvent::ActionDenied, "t2");

    let snap = m.beta_exit_snapshot();
    assert_eq!(snap.action_approved, 2);
    assert_eq!(snap.action_denied, 1);
}

// ── Export format ────────────────────────────────────────────────────

#[test]
fn export_v1_roundtrip_with_profile_and_artifacts() {
    let mut builder = ExportBuilder::new("tenant-export-1");
    builder.add_profile("name: Test User\nsummary: ML Engineer\nskills:\n- Rust\n- Python\n");
    builder.add_records(
        "listings",
        &serde_json::json!([
            {"id": "l1", "title": "Senior ML Engineer", "source": "greenhouse"},
            {"id": "l2", "title": "Robotics Lead", "source": "lever"}
        ]),
    );
    builder.add_records(
        "outcomes",
        &serde_json::json!([
            {"id": "o1", "type": "interview", "status": "completed"}
        ]),
    );
    builder.add_artifact("resume_v1.pdf", b"%PDF-1.4 resume content");
    builder.add_artifact("cover_letter_v1.docx", b"PK cover letter docx");

    let archive = builder.build().unwrap();
    assert!(!archive.is_empty());

    let manifest = careerai_hosted::ops::export_format::read_export(&archive).unwrap();
    assert_eq!(manifest.schema_version, SCHEMA_VERSION);
    assert_eq!(manifest.tenant_id, "tenant-export-1");
    assert_eq!(manifest.entries.len(), 5);

    // Verify entry paths
    let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"profile/profile.yaml"));
    assert!(paths.contains(&"records/listings.json"));
    assert!(paths.contains(&"records/outcomes.json"));
    assert!(paths.contains(&"artifacts/resume_v1.pdf"));
    assert!(paths.contains(&"artifacts/cover_letter_v1.docx"));
}

#[test]
fn export_v1_empty_archive_is_valid() {
    let builder = ExportBuilder::new("tenant-empty");
    let archive = builder.build().unwrap();
    let manifest = careerai_hosted::ops::export_format::read_export(&archive).unwrap();
    assert_eq!(manifest.entries.len(), 0);
    assert!(!manifest.merkle_root.is_empty());
}

#[test]
fn export_v1_merkle_root_changes_with_content() {
    let mut b1 = ExportBuilder::new("t1");
    b1.add_profile("name: Alice");

    let mut b2 = ExportBuilder::new("t1");
    b2.add_profile("name: Bob");

    let m1 = careerai_hosted::ops::export_format::read_export(&b1.build().unwrap()).unwrap();
    let m2 = careerai_hosted::ops::export_format::read_export(&b2.build().unwrap()).unwrap();

    assert_ne!(m1.merkle_root, m2.merkle_root);
}

// ── Artifact object store ─────────────────────────────────────────────

#[tokio::test]
async fn artifact_store_full_lifecycle() {
    let store = ArtifactStore::new(b"test-cap-key-32-bytes-long-enough!".to_vec());

    // Store an artifact
    let meta = store
        .store(
            "t1",
            "resume-001",
            "application/pdf",
            b"%PDF resume content",
        )
        .await;
    assert_eq!(meta.tenant_id, "t1");
    assert_eq!(meta.content_type, "application/pdf");
    assert!(!meta.sha256.is_empty());

    // Issue a capability
    let cap = store
        .issue_capability("t1", &meta.version_id, "u1", "download", 300)
        .await
        .unwrap();

    // Download with capability
    let (downloaded_meta, bytes) = store.download(&cap).await.unwrap();
    assert_eq!(bytes, b"%PDF resume content");
    assert_eq!(downloaded_meta.version_id, meta.version_id);

    // List tenant artifacts
    let artifacts = store.list_for_tenant("t1").await;
    assert_eq!(artifacts.len(), 1);

    // Delete
    store.delete("t1", &meta.version_id).await.unwrap();

    // Orphan scan finds deleted
    let orphans = store.orphan_scan().await;
    assert!(orphans.contains(&meta.version_id));

    // Purge removes deleted
    let purged = store.purge_deleted().await;
    assert_eq!(purged, 1);
    assert!(store.list_for_tenant("t1").await.is_empty());
}

#[tokio::test]
async fn artifact_store_cross_tenant_isolation() {
    let store = ArtifactStore::new(b"test-cap-key-32-bytes-long-enough!".to_vec());

    let meta_t1 = store
        .store("t1", "obj-1", "text/plain", b"tenant 1 data")
        .await;
    let _meta_t2 = store
        .store("t2", "obj-2", "text/plain", b"tenant 2 data")
        .await;

    // t2 cannot issue capability for t1's object
    let result = store
        .issue_capability("t2", &meta_t1.version_id, "u2", "download", 300)
        .await;
    assert!(result.is_err());

    // t1's list doesn't show t2's objects
    let t1_artifacts = store.list_for_tenant("t1").await;
    assert_eq!(t1_artifacts.len(), 1);
    assert_eq!(t1_artifacts[0].tenant_id, "t1");
}

#[tokio::test]
async fn artifact_store_capability_expiry() {
    let store = ArtifactStore::new(b"test-cap-key-32-bytes-long-enough!".to_vec());
    let meta = store.store("t1", "obj-1", "text/plain", b"data").await;

    // Issue with 1-second TTL
    let cap = store
        .issue_capability("t1", &meta.version_id, "u1", "download", 1)
        .await
        .unwrap();

    // Download works immediately
    let result = store.download(&cap).await;
    assert!(result.is_ok());

    // Wait for expiry
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Download fails after expiry
    let result = store.download(&cap).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn artifact_store_forged_capability_rejected() {
    let store = ArtifactStore::new(b"test-cap-key-32-bytes-long-enough!".to_vec());
    let meta = store.store("t1", "obj-1", "text/plain", b"data").await;

    let mut cap = store
        .issue_capability("t1", &meta.version_id, "u1", "download", 300)
        .await
        .unwrap();

    // Tamper with tenant_id in capability
    cap.tenant_id = "t2".to_string();
    let result = store.download(&cap).await;
    assert!(result.is_err());
}

// ── Billing adapter ───────────────────────────────────────────────────

#[tokio::test]
async fn mock_billing_adapter_full_flow() {
    let adapter = MockBillingAdapter::new();

    let req = CheckoutSessionRequest {
        plan_id: "pro".to_string(),
        tenant_id: "t1".to_string(),
        customer_email: Some("user@example.com".to_string()),
        success_url: "https://app.example.com/success".to_string(),
        cancel_url: "https://app.example.com/cancel".to_string(),
    };

    let session = adapter.create_checkout(req).await.unwrap();
    assert!(session.checkout_url.contains("plan=pro"));
    assert_eq!(session.provider, "mock");

    // No active subscription initially
    let sub = adapter.get_subscription("t1").await.unwrap();
    assert!(sub.is_none());

    // Cancel succeeds
    adapter.cancel_subscription("sub_123").await.unwrap();
}

#[tokio::test]
async fn unconfigured_stripe_adapter_returns_not_implemented() {
    let adapter = StripeBillingAdapter::new(None, None);
    assert!(!adapter.is_configured());

    let req = CheckoutSessionRequest {
        plan_id: "pro".to_string(),
        tenant_id: "t1".to_string(),
        customer_email: None,
        success_url: "https://app.example.com/success".to_string(),
        cancel_url: "https://app.example.com/cancel".to_string(),
    };

    let result = adapter.create_checkout(req).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn configured_stripe_adapter_creates_checkout_url() {
    let adapter = StripeBillingAdapter::new(
        Some("sk_test_fake_key".to_string()),
        Some("whsec_fake_secret".to_string()),
    );
    assert!(adapter.is_configured());
    assert_eq!(adapter.provider_name(), "stripe");

    let req = CheckoutSessionRequest {
        plan_id: "pro".to_string(),
        tenant_id: "t1".to_string(),
        customer_email: Some("user@example.com".to_string()),
        success_url: "https://app.example.com/success".to_string(),
        cancel_url: "https://app.example.com/cancel".to_string(),
    };

    let session = adapter.create_checkout(req).await.unwrap();
    assert_eq!(session.provider, "stripe");
    assert!(session.checkout_url.contains("checkout.stripe.com"));
}

// ── API route tests for ops endpoints ─────────────────────────────────

mod api_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use careerai_hosted::api::routes::router;
    use careerai_hosted::api::state::AppState;
    use tower::ServiceExt;

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_format() {
        let state = AppState::arc([0u8; 32]);
        state.metrics.record(
            careerai_hosted::ops::metrics::MetricEvent::OnboardingCompleted,
            "t1",
        );
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_endpoint_returns_200() {
        let state = AppState::arc([0u8; 32]);
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
