//! Integration test: referral endpoint returns matches when profile
//! experience companies overlap with listing companies.
//!
//! The `api_referrals` handler loads the profile from `CAREERAI_ROOT`,
//! extracts past companies, fetches shortlisted+ listings from the DB,
//! and calls `find_referral_opportunities`. This test exercises the
//! full chain through the HTTP endpoint.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use careerai_core::state::ListingState;
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

/// Serialize tests that set `CAREERAI_ROOT` so they don't race.
/// Uses tokio::sync::Mutex because the guard is held across .await points.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn referral_endpoint_returns_match_for_past_employer() {
    let _guard = ENV_LOCK.lock().await;

    // Create a temp dir with a profile whose experience includes "Acme Corp".
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("profile")).unwrap();
    std::fs::write(
        tmp.path().join("profile").join("profile.yaml"),
        "personal:\n  name: \"Referral Tester\"\n  email: \"ref@test.com\"\nsummary: \"Engineer\"\nskills:\n  languages:\n    - \"Rust\"\nexperience:\n  - title: \"Senior Engineer\"\n    company: \"Acme Corp\"\n    location: \"Remote\"\n    start: \"2020-01\"\n    end: \"present\"\n    bullets:\n      - \"Built things at Acme.\"\n",
    )
    .unwrap();

    // Point the handler's profile resolution at our temp dir.
    std::env::set_var("CAREERAI_ROOT", tmp.path());

    // Seed a listing at "Acme Corp" in Shortlisted state.
    let pool = pool_in_memory().await.expect("in-memory pool");
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "ref-1".into(),
            title: "Staff Engineer".into(),
            company: "Acme Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/ref-1".into(),
            description: "Build things.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    queries::transition(&pool, &listing_id, ListingState::Shortlisted, None)
        .await
        .unwrap();

    // Start the dashboard server.
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

    let body = http_get(&format!("http://127.0.0.1:{port}/api/v1/referrals"));

    server.abort();
    std::env::remove_var("CAREERAI_ROOT");

    // Response must contain a referral for Acme Corp.
    assert!(
        body.contains("Acme Corp"),
        "referral response must contain 'Acme Corp': {body}"
    );
    assert!(
        body.contains("Staff Engineer"),
        "referral response must contain the listing title: {body}"
    );
    assert!(
        body.contains("connection_source"),
        "referral response must include connection_source field: {body}"
    );
    assert!(
        body.contains("Previously worked at"),
        "connection_source must mention past employment: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn referral_endpoint_returns_empty_when_no_overlap() {
    let _guard = ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("profile")).unwrap();
    std::fs::write(
        tmp.path().join("profile").join("profile.yaml"),
        "personal:\n  name: \"No Overlap\"\n  email: \"no@test.com\"\nsummary: \"Engineer\"\nskills:\n  languages:\n    - \"Rust\"\nexperience:\n  - title: \"Engineer\"\n    company: \"UnknownCo\"\n    location: \"Remote\"\n    start: \"2020-01\"\n    end: \"present\"\n    bullets:\n      - \"Worked.\"\n",
    )
    .unwrap();

    std::env::set_var("CAREERAI_ROOT", tmp.path());

    let pool = pool_in_memory().await.expect("in-memory pool");
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "ref-2".into(),
            title: "Engineer".into(),
            company: "DifferentCorp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/ref-2".into(),
            description: "Build things.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();
    queries::transition(&pool, &listing_id, ListingState::Shortlisted, None)
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
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let body = http_get(&format!("http://127.0.0.1:{port}/api/v1/referrals"));

    server.abort();
    std::env::remove_var("CAREERAI_ROOT");

    // No overlap → empty array.
    assert!(
        body.trim() == "[]" || body.contains("[]"),
        "referral response should be empty array: {body}"
    );
}
