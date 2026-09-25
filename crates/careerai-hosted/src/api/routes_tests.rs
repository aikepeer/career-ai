//! Integration tests for the API routes.

use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::api::state::AppState;
use crate::auth::session::SessionManager;
use crate::auth::totp::Totp;

/// Create a state with a pre-authenticated session for owner role.
/// Returns (state, session_token).
async fn state_with_session() -> (std::sync::Arc<AppState>, String) {
    let state = AppState::arc([42u8; 32]);
    let (raw, record) = SessionManager::create("t1", "u1", "w1", "owner");
    state
        .sessions
        .write()
        .await
        .insert(record.token_hash.clone(), record);
    state.seed_entitlement("t1", 10_000).await;
    (state, raw)
}

fn json_request(method: &str, uri: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("user-agent", "test-agent")
        .header("x-real-ip", "127.0.0.1")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn authed_request(
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("user-agent", "test-agent")
        .header("x-real-ip", "127.0.0.1");
    let _ = &mut builder;
    if let Some(b) = body {
        builder.body(Body::from(b.to_string())).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    }
}

async fn body_to_json(body: Body) -> serde_json::Value {
    use axum::body::to_bytes;
    let bytes = to_bytes(body, 1_000_000).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ── Health ───────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_200() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── Auth flow ────────────────────────────────────────────────────────

#[tokio::test]
async fn start_login_returns_magic_link() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);
    let resp = app
        .oneshot(json_request(
            "POST",
            "/v1/auth/start",
            serde_json::json!({"email": "test@example.com"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_to_json(resp.into_body()).await;
    assert!(body.get("magic_link_token").is_some());
    assert!(body.get("login_tx_id").is_some());
}

#[tokio::test]
async fn login_callback_enrolls_totp_on_first_login() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);

    let resp = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/auth/start",
            serde_json::json!({"email": "user@example.com"}),
        ))
        .await
        .unwrap();
    let body = body_to_json(resp.into_body()).await;
    let magic_token = body["magic_link_token"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(json_request(
            "POST",
            "/v1/auth/callback",
            serde_json::json!({"token": magic_token}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_to_json(resp.into_body()).await;
    assert_eq!(body["status"], "totp_enrollment");
    assert!(body.get("base32_secret").is_some());
}

#[tokio::test]
async fn full_auth_flow_creates_session() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);

    // 1. Start login
    let resp = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/auth/start",
            serde_json::json!({"email": "full@example.com"}),
        ))
        .await
        .unwrap();
    let body = body_to_json(resp.into_body()).await;
    let magic1 = body["magic_link_token"].as_str().unwrap().to_string();

    // 2. Enroll TOTP
    let resp = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/auth/callback",
            serde_json::json!({"token": magic1}),
        ))
        .await
        .unwrap();
    let body = body_to_json(resp.into_body()).await;
    let b32_secret = body["base32_secret"].as_str().unwrap().to_string();

    // 3. Start login again (new magic link)
    let resp = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/v1/auth/start",
            serde_json::json!({"email": "full@example.com"}),
        ))
        .await
        .unwrap();
    let body = body_to_json(resp.into_body()).await;
    let magic2 = body["magic_link_token"].as_str().unwrap().to_string();

    // 4. Verify TOTP + create session
    let totp = Totp::from_base32(&b32_secret).unwrap();
    let code = totp.now();
    let resp = app
        .oneshot(json_request(
            "POST",
            "/v1/auth/callback",
            serde_json::json!({"token": magic2, "totp_code": code}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_to_json(resp.into_body()).await;
    assert_eq!(body["status"], "authenticated");
    assert!(body.get("session_token").is_some());
    assert!(body.get("workspace_id").is_some());
    assert!(body.get("recovery_codes").is_some());
}

#[tokio::test]
async fn login_callback_rejects_bad_token() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);
    let resp = app
        .oneshot(json_request(
            "POST",
            "/v1/auth/callback",
            serde_json::json!({"token": "nonexistent"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── Protected endpoints ──────────────────────────────────────────────

#[tokio::test]
async fn protected_endpoint_without_session_returns_401() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/workspaces")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_listings_returns_empty() {
    let (state, token) = state_with_session().await;
    let app = router(state);
    let resp = app
        .oneshot(authed_request("GET", "/v1/listings", &token, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_to_json(resp.into_body()).await;
    assert!(body.is_array());
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn billing_checkout_returns_url() {
    let (state, token) = state_with_session().await;
    let app = router(state);
    let resp = app
        .oneshot(authed_request(
            "POST",
            "/v1/billing/checkout",
            &token,
            Some(serde_json::json!({"plan_id": "pro"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_to_json(resp.into_body()).await;
    assert!(body.get("checkout_url").is_some());
}

#[tokio::test]
async fn create_export_with_fresh_session_succeeds() {
    let (state, token) = state_with_session().await;
    let app = router(state);
    let resp = app
        .oneshot(authed_request("POST", "/v1/exports", &token, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_to_json(resp.into_body()).await;
    assert!(body.get("ciphertext_hex").is_some());
    assert!(body.get("nonce_hex").is_some());
}

#[tokio::test]
async fn delete_account_revokes_session() {
    let (state, token) = state_with_session().await;
    let app = router(state.clone());
    let resp = app
        .clone()
        .oneshot(authed_request("DELETE", "/v1/account", &token, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .oneshot(authed_request("GET", "/v1/workspaces", &token, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_session() {
    let (state, token) = state_with_session().await;
    let app = router(state.clone());
    let resp = app
        .clone()
        .oneshot(authed_request("POST", "/v1/auth/logout", &token, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .oneshot(authed_request("GET", "/v1/workspaces", &token, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── Billing webhooks ─────────────────────────────────────────────────

fn compute_sig(payload: &str, secret: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[tokio::test]
async fn webhook_rejects_missing_signature() {
    let state = AppState::arc([0u8; 32]);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/billing/webhooks/stripe")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"id":"evt_1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_accepts_valid_signature() {
    let state = AppState::arc([0u8; 32]);
    let secret = b"whsec_test";
    state.set_webhook_secret("stripe", secret.to_vec()).await;

    let payload = r#"{"id":"evt_ok","type":"payment.succeeded","version":1,"customer":"c1","subscription":"s1"}"#;
    let sig = compute_sig(payload, secret);

    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/billing/webhooks/stripe")
                .header("content-type", "application/json")
                .header("x-webhook-signature", &sig)
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn webhook_rejects_bad_signature() {
    let state = AppState::arc([0u8; 32]);
    state
        .set_webhook_secret("stripe", b"whsec_test".to_vec())
        .await;

    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/billing/webhooks/stripe")
                .header("content-type", "application/json")
                .header("x-webhook-signature", "deadbeef")
                .body(Body::from(r#"{"id":"evt_1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_rejects_duplicate_event() {
    let state = AppState::arc([0u8; 32]);
    let secret = b"whsec_test";
    state.set_webhook_secret("stripe", secret.to_vec()).await;

    let payload = r#"{"id":"evt_dup","type":"payment.succeeded","version":1,"customer":"c1","subscription":"s1"}"#;
    let sig = compute_sig(payload, secret);

    // First: OK
    let app = router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/billing/webhooks/stripe")
                .header("x-webhook-signature", &sig)
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Second: conflict
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/billing/webhooks/stripe")
                .header("x-webhook-signature", &sig)
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
