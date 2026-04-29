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

    // Give the server a moment to bind.
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
        body.contains("Next steps"),
        "body missing next-steps section"
    );

    server.abort();
}

async fn reqwest_get(url: &str) -> String {
    // Shell out to curl. Keeps the test dep-free and avoids subtle
    // hand-rolled-HTTP edge cases against the real axum server.
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
