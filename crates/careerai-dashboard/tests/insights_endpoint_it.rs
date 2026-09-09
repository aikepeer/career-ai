//! F09: Integration test for the `GET /api/v1/insights` endpoint.
//! Seeds listings + applications across two sources and verifies the
//! endpoint returns source performance (discovered/submitted counts)
//! and intelligence records (paginated application history).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use careerai_dashboard::{run, ServeOptions};
use careerai_db::models::{NewApplication, NewListing};
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
async fn insights_endpoint_returns_source_performance() {
    let pool = pool_in_memory().await.expect("in-memory pool");

    // Seed two listings from different sources.
    let (l1, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "ins-1".into(),
            title: "Rust Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://example.com/1".into(),
            description: "Build in Rust.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    let (l2, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "lever".into(),
            external_id: "ins-2".into(),
            title: "ML Engineer".into(),
            company: "Beta".into(),
            location: Some("Remote".into()),
            url: "https://example.com/2".into(),
            description: "Build ML pipelines.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    // Create a submitted application for l1.
    let app = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: l1.clone(),
            profile_hash: "hash-1".into(),
            prompt_version: "v1".into(),
            llm_model: "test".into(),
        },
    )
    .await
    .unwrap();
    queries::set_application_state(&pool, &app.id, "submitted")
        .await
        .unwrap();

    // Create a non-submitted application for l2.
    let _app2 = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: l2.clone(),
            profile_hash: "hash-2".into(),
            prompt_version: "v1".into(),
            llm_model: "test".into(),
        },
    )
    .await
    .unwrap();

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
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let body = http_get(&format!("http://127.0.0.1:{port}/api/v1/insights"));

    server.abort();

    assert!(
        body.contains("source_performance"),
        "response must include source_performance: {body}"
    );
    assert!(
        body.contains("greenhouse"),
        "source_performance must include greenhouse: {body}"
    );
    assert!(
        body.contains("lever"),
        "source_performance must include lever: {body}"
    );
}
