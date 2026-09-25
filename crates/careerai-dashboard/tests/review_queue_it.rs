//! F03: Integration test for the review queue API endpoints.
//! Verifies the queue lists reviewable applications and approve/skip/retry
//! actions enforce content-version safety.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use careerai_dashboard::{run, ServeOptions};
use careerai_db::models::NewListing;
use careerai_db::{pool::pool_in_memory, queries};

async fn pick_free_port() -> u16 {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

fn http_get(url: &str) -> String {
    let output = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "10", url])
        .output()
        .expect("curl failed");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn http_post(url: &str, body: &str) -> String {
    let output = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "10",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            body,
            url,
        ])
        .output()
        .expect("curl failed");
    String::from_utf8_lossy(&output.stdout).to_string()
}

async fn wait_for_server(port: u16) {
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn seed_rendered_application(pool: &sqlx::SqlitePool) -> String {
    let (listing_id, _) = queries::insert_or_ignore(
        pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "rq-1".into(),
            title: "Rust Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://example.com/rq-1".into(),
            description: "Build systems in Rust.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    queries::transition(
        pool,
        &listing_id,
        careerai_core::state::ListingState::Shortlisted,
        None,
    )
    .await
    .unwrap();
    queries::transition(
        pool,
        &listing_id,
        careerai_core::state::ListingState::Tailored,
        None,
    )
    .await
    .unwrap();
    queries::transition(
        pool,
        &listing_id,
        careerai_core::state::ListingState::Rendered,
        None,
    )
    .await
    .unwrap();

    // Create an application in the rendered state.
    let app_id = format!("app_{listing_id}");
    sqlx::query(
        "INSERT INTO applications (id, listing_id, state, profile_hash, prompt_version, llm_model, created_at, updated_at) \
         VALUES (?, ?, 'rendered', 'sha256:abc', 'v1', 'none', ?, ?)",
    )
    .bind(&app_id)
    .bind(&listing_id)
    .bind(chrono::Utc::now())
    .bind(chrono::Utc::now())
    .execute(pool)
    .await
    .unwrap();

    app_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_queue_lists_rendered_applications() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    let app_id = seed_rendered_application(&pool).await;

    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool: pool.clone(),
    };
    let server = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    wait_for_server(port).await;

    let body = http_get(&format!("http://127.0.0.1:{port}/api/v1/review-queue"));
    server.abort();

    assert!(
        body.contains(&app_id),
        "review queue must list the rendered application: {body}"
    );
    assert!(
        body.contains("Rust Engineer"),
        "review queue must include the job title: {body}"
    );
    assert!(
        body.contains("\"profile_hash\""),
        "review queue must include profile_hash for version tracking: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_approve_rejects_stale_content_version() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    let app_id = seed_rendered_application(&pool).await;

    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool: pool.clone(),
    };
    let server = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    wait_for_server(port).await;

    // Submit an approval with a deliberately stale content version.
    let stale_body = format!(
        r#"{{"content_version":{{"application_id":"{app_id}","profile_hash":"sha256:WRONG","prompt_version":"v1","updated_at_rfc3339":"2020-01-01T00:00:00Z"}},"note":null}}"#
    );
    let resp = http_post(
        &format!("http://127.0.0.1:{port}/api/v1/review-queue/{app_id}/approve"),
        &stale_body,
    );
    server.abort();

    assert!(
        resp.contains("stale_content"),
        "stale content version must be rejected: {resp}"
    );
}
