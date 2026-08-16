#![allow(clippy::expect_used, clippy::unused_async)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use careerai_dashboard::{run, ServeOptions};
use careerai_db::pool::pool_in_memory;

async fn pick_free_port() -> u16 {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_and_index_render() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool: pool.clone(),
    };

    let server = tokio::spawn(async move {
        match run(opts).await {
            Ok(()) => eprintln!("server exited cleanly"),
            Err(e) => eprintln!("server error: {e:#}"),
        }
    });

    let mut connected = false;
    for i in 0..40 {
        match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            Ok(_) => {
                connected = true;
                eprintln!("connected on attempt {i}");
                break;
            }
            Err(e) if i % 5 == 0 => eprintln!("attempt {i}: {e}"),
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        connected,
        "server never accepted a connection on port {port}"
    );

    let healthz = reqwest_get(&format!("http://127.0.0.1:{port}/healthz")).await;
    assert!(healthz.starts_with("ok"), "healthz body: {healthz}");

    let body = reqwest_get(&format!("http://127.0.0.1:{port}/")).await;
    assert!(
        body.contains("career-ai"),
        "body missing brand: {body:.300}"
    );
    assert!(
        body.contains("kpi-strip"),
        "body missing kpi-strip role: {body:.300}"
    );
    assert!(
        body.contains("Pipeline & Funnel"),
        "body missing tab label"
    );

    let snapshot_json = reqwest_get(&format!("http://127.0.0.1:{port}/api/v1/snapshot")).await;
    assert!(snapshot_json.contains("kpi"), "snapshot endpoint response invalid: {snapshot_json}");

    let events_json = reqwest_get(&format!("http://127.0.0.1:{port}/api/v1/events")).await;
    assert!(events_json.starts_with('['), "events endpoint response invalid: {events_json}");

    let config_json = reqwest_get(&format!("http://127.0.0.1:{port}/api/v1/config")).await;
    assert!(config_json.contains("score_threshold"), "config endpoint response invalid: {config_json}");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_api_endpoints_respond() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool: pool.clone(),
    };

    let server = tokio::spawn(async move {
        match run(opts).await {
            Ok(()) => eprintln!("server exited cleanly"),
            Err(e) => eprintln!("server error: {e:#}"),
        }
    });

    // Wait for server
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let base = format!("http://127.0.0.1:{port}");

    // POST /api/v1/config/generate — should succeed or fail with 500, never hang
    let gen = curl_post_json(&format!("{base}/api/v1/config/generate"), "{}").await;
    assert!(
        gen.contains("success") || gen.contains("error"),
        "config/generate unexpected: {gen}"
    );

    // POST /api/v1/config/prompt — extract keywords from a natural-language prompt
    let prompt_body = r#"{"prompt":"Focus on Remote Embedded Linux and Freelance AI jobs"}"#;
    let prompt = curl_post_json(&format!("{base}/api/v1/config/prompt"), prompt_body).await;
    assert!(
        prompt.contains("extracted_keywords"),
        "config/prompt unexpected: {prompt}"
    );

    // POST /api/v1/profile/import — missing file field → 400 with error
    let import = curl_post_json(&format!("{base}/api/v1/profile/import"), "{}").await;
    // multipart not sent via JSON → expect an error, not a panic
    assert!(
        import.contains("error") || import.contains("No file"),
        "profile/import unexpected: {import}"
    );

    // POST /api/v1/listings/<id>/shortlist — unknown id → 500, not crash
    let shortlist = curl_post_json(
        &format!("{base}/api/v1/listings/nonexistent/shortlist"),
        "{}",
    )
    .await;
    assert!(
        shortlist.contains("success") || shortlist.contains("error"),
        "shortlist unexpected: {shortlist}"
    );

    // GET /api/v1/explorer — should return JSON array
    let explorer = reqwest_get(&format!("{base}/api/v1/explorer")).await;
    assert!(
        explorer.starts_with('['),
        "explorer endpoint unexpected: {explorer}"
    );

    server.abort();
}

async fn curl_post_json(url: &str, body: &str) -> String {
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
        .expect("curl failed to spawn");
    String::from_utf8_lossy(&output.stdout).to_string()
}

async fn reqwest_get(url: &str) -> String {
    let output = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "10", url])
        .output()
        .expect("curl failed to spawn");
    assert!(
        output.status.success(),
        "curl {url} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}
