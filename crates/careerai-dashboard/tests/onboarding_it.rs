//! F01: Integration test for the `GET /api/v1/onboarding` endpoint.
//! Verifies the onboarding phase and preview matches are returned correctly.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn onboarding_returns_import_phase_for_empty_workspace() {
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
    wait_for_server(port).await;

    let body = http_get(&format!("http://127.0.0.1:{port}/api/v1/onboarding"));
    server.abort();

    assert!(
        body.contains("\"import_profile\""),
        "empty workspace must be in import_profile phase: {body}"
    );
    assert!(
        body.contains("\"has_profile\":false"),
        "empty workspace must report has_profile false: {body}"
    );
    assert!(
        body.contains("\"config\""),
        "next_action_tab must be config: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn onboarding_returns_preview_matches_when_listings_exist() {
    let pool = pool_in_memory().await.expect("in-memory pool");

    // Seed two shortlisted listings with scores.
    for i in 0..2 {
        let (id, _) = queries::insert_or_ignore(
            &pool,
            &NewListing {
                source: "greenhouse".into(),
                external_id: format!("onb-{i}"),
                title: format!("AI Engineer {i}"),
                company: format!("Corp {i}"),
                location: Some("Remote".into()),
                url: format!("https://example.com/onb-{i}"),
                description: "Build AI systems.".into(),
                raw_json: None,
            },
        )
        .await
        .unwrap();
        queries::transition(&pool, &id, careerai_core::state::ListingState::Shortlisted, None)
            .await
            .unwrap();
        sqlx::query("UPDATE listings SET score = ? WHERE id = ?")
            .bind(0.9 - f64::from(i) * 0.1)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
    }

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

    let body = http_get(&format!("http://127.0.0.1:{port}/api/v1/onboarding"));
    server.abort();

    assert!(
        body.contains("preview_matches"),
        "response must include preview_matches: {body}"
    );
    assert!(
        body.contains("AI Engineer 0"),
        "preview must include the first seeded listing: {body}"
    );
}
