//! F02: Integration test for the `GET /api/v1/match-reasons/:listing_id`
//! endpoint. Seeds a listing, upserts match reasons, and verifies the
//! endpoint returns the structured breakdown as JSON.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn match_reasons_endpoint_returns_persisted_reasons() {
    let pool = pool_in_memory().await.expect("in-memory pool");

    // Seed a listing.
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "mr-ep-1".into(),
            title: "Rust Engineer".into(),
            company: "Acme Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/mr-ep-1".into(),
            description: "Build distributed systems in Rust.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    // Persist match reasons for it.
    queries::upsert_match_reasons(
        &pool,
        &listing_id,
        0.82,
        r#"["rust","tokio","async"]"#,
        r#"["grpc"]"#,
        None,
        Some("legit"),
        Some(0.9),
        None,
    )
    .await
    .unwrap();

    // Start the dashboard server with the in-memory pool.
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

    // Wait for server to be ready.
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let body = http_get(&format!(
        "http://127.0.0.1:{port}/api/v1/match-reasons/{listing_id}"
    ));

    server.abort();

    assert!(
        body.contains("\"matched_keywords\""),
        "response must include matched_keywords: {body}"
    );
    assert!(
        body.contains("rust"),
        "matched_keywords must contain 'rust': {body}"
    );
    assert!(
        body.contains("\"missing_keywords\""),
        "response must include missing_keywords: {body}"
    );
    assert!(
        body.contains("grpc"),
        "missing_keywords must contain 'grpc': {body}"
    );
    assert!(
        body.contains("\"legitimacy_tier\""),
        "response must include legitimacy_tier: {body}"
    );
    assert!(
        body.contains("legit"),
        "legitimacy_tier must be 'legit': {body}"
    );
    assert!(
        body.contains("0.819"),
        "response must include the score ~0.82: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn match_reasons_endpoint_returns_404_for_unknown_listing() {
    let pool = pool_in_memory().await.expect("in-memory pool");

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

    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let url = format!("http://127.0.0.1:{port}/api/v1/match-reasons/nonexistent-listing");
    let output = std::process::Command::new("curl")
        .args(["-sS", "-o", "/dev/null", "-w", "%{http_code}", "--max-time", "10", &url])
        .output()
        .expect("curl failed");
    let status = String::from_utf8_lossy(&output.stdout).to_string();

    server.abort();

    assert_eq!(
        status, "404",
        "unknown listing should return 404, got {status}"
    );
}
