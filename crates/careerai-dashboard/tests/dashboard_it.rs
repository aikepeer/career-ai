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

/// Insert a listing and return its generated DB id, so served-HTML
/// assertions can target the real id.
async fn seed_listing_id(pool: &sqlx::SqlitePool) -> String {
    use careerai_db::models::NewListing;
    use careerai_db::queries::listings::insert_or_ignore;
    let (id, _) = insert_or_ignore(
        pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "explorer-it-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/explorer-it-1".into(),
            description: "Build LLM systems".into(),
            raw_json: None,
        },
    )
    .await
    .expect("insert listing");
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_and_index_render() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    // Seed one listing so the server-rendered Explorer table has a row
    // whose copyable id cell can be asserted in the served HTML.
    let listing_id = seed_listing_id(&pool).await;
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
    assert!(body.contains("Mission Control"), "body missing tab label");
    assert!(
        body.contains("runCliCommand"),
        "body missing generic CLI dispatcher hook: {body:.300}"
    );
    assert!(body.contains("Run All"), "body missing Run All control");
    assert!(
        body.contains("CLI Commands"),
        "body missing CLI Commands tab: {body:.300}"
    );
    assert!(
        body.contains("guided-path"),
        "body missing numbered guided path: {body:.300}"
    );
    assert!(
        body.contains("careerai init"),
        "body missing init in command catalog: {body:.300}"
    );
    assert!(
        body.contains("careerai daemon"),
        "body missing daemon in command catalog: {body:.300}"
    );
    assert!(
        body.contains("listing-id-cell"),
        "explorer rows must render a copyable id cell: {body:.2000}"
    );
    assert!(
        body.contains(&listing_id),
        "explorer must render the seeded listing id {listing_id}: {body:.2000}"
    );

    let snapshot_json = reqwest_get(&format!("http://127.0.0.1:{port}/api/v1/snapshot")).await;
    assert!(
        snapshot_json.contains("kpi"),
        "snapshot endpoint response invalid: {snapshot_json}"
    );

    let events_json = reqwest_get(&format!("http://127.0.0.1:{port}/api/v1/events")).await;
    assert!(
        events_json.starts_with('['),
        "events endpoint response invalid: {events_json}"
    );

    let config_json = reqwest_get(&format!("http://127.0.0.1:{port}/api/v1/config")).await;
    assert!(
        config_json.contains("score_threshold"),
        "config endpoint response invalid: {config_json}"
    );

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
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let base = format!("http://127.0.0.1:{port}");

    // POST /api/v1/config/generate — now returns a non-destructive preview
    let gen = curl_post_json(&format!("{base}/api/v1/config/generate"), "{}").await;
    assert!(
        gen.contains("preview") || gen.contains("error"),
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
    // A JSON body against the multipart extractor is rejected by axum with
    // a boundary error (a 400-class rejection, not a panic). Accept either
    // that rejection text or our own "No files uploaded" message.
    assert!(
        import.contains("error") || import.contains("No file") || import.contains("boundary"),
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

    // POST /api/v1/chat — interactive LLM assistant endpoint
    std::env::set_var("CAREERAI_DISABLE_AGY", "1");
    let chat_body = r#"{"message":"Analyze my current job match pipeline statistics"}"#;
    let chat_res = curl_post_json(&format!("{base}/api/v1/chat"), chat_body).await;
    std::env::remove_var("CAREERAI_DISABLE_AGY");
    assert!(
        chat_res.contains("reply") || chat_res.contains("status"),
        "chat endpoint unexpected: {chat_res}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutating_endpoints_reject_cross_origin() {
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

    let base = format!("http://127.0.0.1:{port}");
    let body = r#"{"prompt":"focus on rust"}"#;

    // A cross-origin browser form/fetch POST must be rejected before the
    // handler runs.
    let (status, resp) = curl_post_json_with_headers(
        &format!("{base}/api/v1/config/prompt"),
        body,
        &["Origin: http://evil.example"],
    )
    .await;
    assert_eq!(status, 403, "cross-origin POST must be 403: {resp}");
    assert!(
        resp.contains("cross-origin"),
        "expected cross-origin rejection body: {resp}"
    );

    // Same-origin requests (matching Host) are allowed.
    let (status, resp) = curl_post_json_with_headers(
        &format!("{base}/api/v1/config/prompt"),
        body,
        &[&format!("Origin: http://127.0.0.1:{port}")],
    )
    .await;
    assert_eq!(status, 200, "same-origin POST must be 200: {resp}");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_shortlist_respects_state_machine() {
    use careerai_core::state::ListingState;
    use careerai_db::models::NewListing;
    use careerai_db::queries::listings::{insert_or_ignore, transition};

    let pool = pool_in_memory().await.expect("in-memory pool");
    let (listing_id, inserted) = insert_or_ignore(
        &pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: "g-1".into(),
            title: "Embedded Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://example.com/g-1".into(),
            description: "Rust firmware".into(),
            raw_json: None,
        },
    )
    .await
    .expect("insert listing");
    assert!(inserted);

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

    let base = format!("http://127.0.0.1:{port}");
    let shortlist_url = format!("{base}/api/v1/listings/{listing_id}/shortlist");

    let first = curl_post_json(&shortlist_url, "{}").await;
    assert!(
        first.contains("success"),
        "discovered -> shortlisted should succeed: {first}"
    );

    let again = curl_post_json(&shortlist_url, "{}").await;
    assert!(
        again.contains("already_shortlisted"),
        "idempotent shortlist should report already_shortlisted: {again}"
    );

    // Move the listing to a terminal state and confirm shortlisting is
    // refused instead of corrupting the pipeline.
    transition(
        &pool,
        &listing_id,
        ListingState::Submitted,
        Some("submitted for test"),
    )
    .await
    .expect("transition to submitted");

    let conflict = curl_post_json(&shortlist_url, "{}").await;
    assert!(
        conflict.contains("cannot shortlist"),
        "submitted -> shortlisted must be refused: {conflict}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drafted_listings_surface_review_control() {
    use careerai_core::state::ListingState;
    use careerai_db::models::NewListing;
    use careerai_db::queries::listings::{insert_or_ignore, transition};

    let pool = pool_in_memory().await.expect("in-memory pool");
    let (listing_id, inserted) = insert_or_ignore(
        &pool,
        &NewListing {
            source: "linkedin".into(),
            external_id: "li-1".into(),
            title: "Embedded Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://example.com/li-1".into(),
            description: "Rust firmware".into(),
            raw_json: None,
        },
    )
    .await
    .expect("insert listing");
    assert!(inserted);
    transition(
        &pool,
        &listing_id,
        ListingState::Drafted,
        Some("drafted for test"),
    )
    .await
    .expect("transition to drafted");

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

    let body = reqwest_get(&format!("http://127.0.0.1:{port}/")).await;
    assert!(
        body.contains("Review Drafted LinkedIn"),
        "drafted count must surface the review control: {body:.400}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_endpoint_whitelist_and_status() {
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

    let base = format!("http://127.0.0.1:{port}");
    let run_url = format!("{base}/api/v1/cli/run");

    // 400 — disallowed process-control command, rejected before any spawn.
    let (status, body) =
        curl_post_json_with_headers(&run_url, r#"{"command":"daemon"}"#, &[]).await;
    assert_eq!(status, 400, "daemon must be rejected: {body}");
    assert!(body.contains("not available"), "expected reason: {body}");

    // 400 — shell metacharacters in an id must never reach a child process.
    let (status, body) = curl_post_json_with_headers(
        &run_url,
        r#"{"command":"tailor","args":{"listing_id":"bad;id"}}"#,
        &[],
    )
    .await;
    assert_eq!(status, 400, "invalid id must be rejected: {body}");
    assert!(body.contains("invalid id"), "expected id error: {body}");

    // 200 — whitelisted command runs a real executable (/bin/true).
    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) =
        curl_post_json_with_headers(&run_url, r#"{"command":"llm probe"}"#, &[]).await;
    assert_eq!(status, 200, "llm probe should succeed: {body}");
    assert!(body.contains("\"status\":\"success\""), "body: {body}");

    // The old pipeline/discover + pipeline/match wrappers still respond.
    let (status, _) =
        curl_post_json_with_headers(&format!("{base}/api/v1/pipeline/discover"), "{}", &[]).await;
    assert_eq!(status, 200, "discover wrapper should respond");
    let (status, _) =
        curl_post_json_with_headers(&format!("{base}/api/v1/pipeline/match"), "{}", &[]).await;
    assert_eq!(status, 200, "match wrapper should respond");

    // 500 — spawn failure (a directory is not executable).
    std::env::set_var("CAREERAI_BIN", "/");
    let (status, body) =
        curl_post_json_with_headers(&run_url, r#"{"command":"llm probe"}"#, &[]).await;
    assert_eq!(status, 500, "spawn failure must be 500: {body}");
    assert!(body.contains("\"status\":\"error\""), "body: {body}");

    // 409 — a slow first run holds the in-process lock; a concurrent run is
    // refused until it finishes.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = tmp.path().join("slow");
        let ready = tmp.path().join("ready");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch \"{}\"\nsleep 2\n", ready.display()),
        )
        .expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
        std::env::set_var("CAREERAI_BIN", &script);

        let first_url = run_url.clone();
        let first =
            tokio::spawn(
                async move { curl_post_json(&first_url, r#"{"command":"llm probe"}"#).await },
            );

        let mut ready_seen = false;
        for _ in 0..40 {
            if ready.exists() {
                ready_seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(ready_seen, "slow script never started");

        let (status, body) =
            curl_post_json_with_headers(&run_url, r#"{"command":"llm probe"}"#, &[]).await;
        assert_eq!(status, 409, "concurrent run must be busy: {body}");

        let first_body = first.await.expect("first run join");
        assert!(
            first_body.contains("\"status\":\"success\""),
            "first run should succeed: {first_body}"
        );
    }

    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

async fn curl_post_json(url: &str, body: &str) -> String {
    let (_, stdout) = curl_post_json_with_headers(url, body, &[]).await;
    stdout
}

async fn curl_post_json_with_headers(url: &str, body: &str, headers: &[&str]) -> (u16, String) {
    let mut cmd = std::process::Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "10",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
    ]);
    for header in headers {
        cmd.arg("-H").arg(header);
    }
    cmd.args(["-d", body, "-w", "\n%{http_code}", "-o", "-", url]);
    let output = cmd.output().expect("curl failed to spawn");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let (resp, code) = stdout.rsplit_once('\n').unwrap_or((stdout.as_str(), ""));
    let status: u16 = code.trim().parse().unwrap_or(0);
    (status, resp.to_string())
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
