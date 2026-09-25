//! F06: Integration test for the saved-views API endpoints.
//! Exercises the full CRUD cycle: create → list → delete.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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

fn http(method: &str, url: &str, body: &str) -> String {
    let output = std::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "10",
            "-X",
            method,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_views_crud_cycle() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;

    let handle = tokio::spawn(async move {
        run(ServeOptions {
            port,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            refresh_seconds: 0,
            pool,
        })
        .await
        .expect("server run");
    });

    // Wait for the server to accept connections (readiness probe).
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Create a saved view.
    let create_body = r#"{"name":"Rust Remote","query":"rust","source":"greenhouse","state":"shortlisted","remote_only":true,"filter_json":{}}"#;
    let resp = http(
        "POST",
        &format!("http://127.0.0.1:{port}/api/v1/saved-views"),
        create_body,
    );
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(parsed["name"], "Rust Remote", "create response: {resp}");

    // List — should include the created view.
    let resp = http(
        "GET",
        &format!("http://127.0.0.1:{port}/api/v1/saved-views"),
        "",
    );
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    let views = parsed["views"].as_array().expect("views array");
    assert_eq!(views.len(), 1, "list response: {resp}");
    assert_eq!(views[0]["name"], "Rust Remote");
    assert_eq!(views[0]["query"], "rust");

    // Update — same name, different query.
    let update_body = r#"{"name":"Rust Remote","query":"rust async","source":"greenhouse","state":"shortlisted","remote_only":true,"filter_json":{}}"#;
    let resp = http(
        "POST",
        &format!("http://127.0.0.1:{port}/api/v1/saved-views"),
        update_body,
    );
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(parsed["name"], "Rust Remote");

    // List again — still one view, but query updated.
    let resp = http(
        "GET",
        &format!("http://127.0.0.1:{port}/api/v1/saved-views"),
        "",
    );
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    let views = parsed["views"].as_array().expect("views array");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0]["query"], "rust async");

    // Delete.
    let resp = http(
        "DELETE",
        &format!("http://127.0.0.1:{port}/api/v1/saved-views/Rust%20Remote"),
        "",
    );
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(parsed["deleted"], true);

    // List — should be empty.
    let resp = http(
        "GET",
        &format!("http://127.0.0.1:{port}/api/v1/saved-views"),
        "",
    );
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    let views = parsed["views"].as_array().expect("views array");
    assert!(views.is_empty());

    handle.abort();
}
