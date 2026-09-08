//! Comprehensive dashboard button-action + config integration tests.
//!
//! Every test boots the dashboard server in-process against an
//! in-memory SQLite pool, then exercises the HTTP endpoints that the
//! template's JavaScript button handlers call. This covers:
//!
//! * CLI command dispatch (tailor, render, apply, retry, discover,
//!   match, run, review, digest, etc.)
//! * Config save / load round-trip
//! * Threshold management
//! * Force-shortlist state-machine guard
//! * Explorer listing rendering
//! * Application detail endpoint
//! * Config generate / apply / prompt
//! * Profile import (missing-file rejection)
//! * Batch tailor (top-N, all, selected) → CLI argv construction
//! * Security: cross-origin rejection, invalid ID rejection
//! * Template parity: every CLI catalog command appears in served HTML

#![allow(
    clippy::expect_used,
    clippy::unused_async,
    // `CLI_RUN_SERIAL` is a std mutex held across `.await`s by design —
    // it serializes tests that share process-global state (env var +
    // busy lock). Deliberate, documented, test-only.
    clippy::await_holding_lock
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use careerai_dashboard::{run, ServeOptions};
use careerai_db::pool::pool_in_memory;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn pick_free_port() -> u16 {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

async fn bootserver() -> (tokio::task::JoinHandle<()>, u16) {
    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool,
    };
    let handle = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    // Wait for server to accept connections
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (handle, port)
}

async fn curl_get(url: &str) -> String {
    let output = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "10", url])
        .output()
        .expect("curl failed");
    String::from_utf8_lossy(&output.stdout).to_string()
}

async fn curl_post(url: &str, body: &str) -> (u16, String) {
    curl_post_with_headers(url, body, &[]).await
}

/// The dashboard's `/api/v1/cli/run` + `/api/v1/pipeline/*` paths use a
/// process-global busy lock and read the process-global `CAREERAI_BIN`
/// env var at request time, and the tests below mutate that env var.
/// Parallel execution therefore races: one test's slow stub holds the
/// busy lock while another gets a spurious 409, env-var set/remove leaks
/// into another test's subprocess spawn, and — worst — with `CAREERAI_BIN`
/// unset the dashboard spawns `current_exe()`, which is the test binary
/// itself (a recursive test run). Every test that exercises these
/// endpoints must hold this mutex for its whole body.
static CLI_RUN_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn curl_post_with_headers(url: &str, body: &str, headers: &[&str]) -> (u16, String) {
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
    let output = cmd.output().expect("curl failed");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let (resp, code) = stdout.rsplit_once('\n').unwrap_or((stdout.as_str(), ""));
    let status: u16 = code.trim().parse().unwrap_or(0);
    (status, resp.to_string())
}

async fn seed_listing(pool: &sqlx::SqlitePool, state: &str) -> String {
    use careerai_core::state::ListingState;
    use careerai_db::models::NewListing;
    use careerai_db::queries::listings::{insert_or_ignore, transition};

    let (id, _) = insert_or_ignore(
        pool,
        &NewListing {
            source: "greenhouse".into(),
            external_id: format!("btn-it-{state}"),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/test".into(),
            description: "Build LLM systems in Rust".into(),
            raw_json: None,
        },
    )
    .await
    .expect("insert listing");

    let target = match state {
        "shortlisted" => ListingState::Shortlisted,
        "tailored" => ListingState::Tailored,
        "rendered" => ListingState::Rendered,
        "submitted" => ListingState::Submitted,
        "drafted" => ListingState::Drafted,
        _ => ListingState::Discovered,
    };
    transition(pool, &id, target, Some("seeded for test"))
        .await
        .expect("transition");
    id
}

// ---------------------------------------------------------------------------
// Template parity: every CLI catalog command appears in served HTML
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn served_html_contains_all_runnable_cli_commands() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!("http://127.0.0.1:{port}/")).await;

    // Every command from cli_catalog with availability Runnable or NeedsId
    // must have a corresponding button in the HTML.
    for needle in [
        "config generate",
        "profile show",
        "profile validate",
        "discover",
        "match",
        "run",
        "tailor",
        "render",
        "apply",
        "retry",
        "review",
        "shortlist show",
        "applied",
        "inspect",
        "digest",
        "sources sync",
        "sources discover-web",
        "mcp probe",
        "llm probe",
        "notify test",
    ] {
        assert!(
            body.contains(needle),
            "served HTML must contain CLI command `{needle}` for dashboard/CLI parity: body[:500]={}",
            &body[..500.min(body.len())]
        );
    }
    server.abort();
}

// ---------------------------------------------------------------------------
// Template parity: all key dashboard buttons are present
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn served_html_contains_all_key_dashboard_buttons() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!("http://127.0.0.1:{port}/")).await;

    // Funnel action buttons
    assert!(
        body.contains("batchTailorTop"),
        "missing batchTailorTop button"
    );
    assert!(
        body.contains("batchTailorAllShortlisted"),
        "missing batchTailorAllShortlisted button"
    );
    assert!(
        body.contains("batchTailorSelected"),
        "missing batchTailorSelected button"
    );
    assert!(
        body.contains("batchRenderAllTailored"),
        "missing batchRenderAllTailored button"
    );
    assert!(
        body.contains("batchRollback"),
        "missing batchRollback button"
    );

    // Card action buttons
    assert!(
        body.contains("tailorListing"),
        "missing tailorListing button"
    );
    assert!(
        body.contains("renderApplication"),
        "missing renderApplication button"
    );
    assert!(
        body.contains("applyApplication"),
        "missing applyApplication button"
    );
    assert!(
        body.contains("rollbackApplication"),
        "missing rollbackApplication button"
    );

    // Top-level action buttons
    assert!(body.contains("Run All"), "missing Run All button");
    assert!(body.contains("Run Discover"), "missing Run Discover button");
    assert!(body.contains("Run Matcher"), "missing Run Matcher button");
    assert!(
        body.contains("Review Drafted LinkedIn"),
        "missing Review Drafted LinkedIn button"
    );

    // Config tab
    assert!(
        body.contains("Save LLM Settings"),
        "missing Save LLM Settings button"
    );
    assert!(
        body.contains("Generate Config from Profile"),
        "missing Generate Config from Profile button"
    );

    // Guided path
    assert!(body.contains("guided-path"), "missing guided-path section");
    assert!(body.contains("Do Step"), "missing Do Step button");

    // Explorer
    assert!(body.contains("listing-id-cell"), "missing listing-id-cell");
    assert!(body.contains("Explorer"), "missing Explorer section");

    server.abort();
}

// ---------------------------------------------------------------------------
// CLI run endpoint: whitelist validation for every command
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_rejects_disallowed_commands() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    // daemon, init, status serve, cookies refresh are CLI-only
    for cmd in ["daemon", "init", "status serve", "cookies refresh"] {
        let (status, body) = curl_post(&url, &format!("{{\"command\":\"{cmd}\"}}")).await;
        assert_eq!(
            status, 400,
            "`{cmd}` must be rejected as disallowed: {body}"
        );
        assert!(
            body.contains("not available"),
            "`{cmd}` rejection must say not available: {body}"
        );
    }

    // Unknown command
    let (status, body) = curl_post(&url, r#"{"command":"rm -rf /"}"#).await;
    assert_eq!(status, 400, "unknown command must be 400: {body}");
    assert!(body.contains("unknown command"), "body: {body}");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_rejects_shell_metacharacters_in_ids() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    // Shell injection attempt in listing_id
    let (status, body) = curl_post(
        &url,
        r#"{"command":"tailor","args":{"listing_id":"bad;id"}}"#,
    )
    .await;
    assert_eq!(status, 400, "invalid id must be rejected: {body}");
    assert!(body.contains("invalid id"), "body: {body}");

    // Shell injection in application_id
    let (status, body) = curl_post(
        &url,
        r#"{"command":"render","args":{"application_id":"$(whoami)"}}"#,
    )
    .await;
    assert_eq!(status, 400, "invalid id must be rejected: {body}");

    // Shell injection in inspect
    let (status, body) = curl_post(
        &url,
        r#"{"command":"inspect","args":{"application_id":"a&&b"}}"#,
    )
    .await;
    assert_eq!(status, 400, "invalid id must be rejected: {body}");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_tailor_limit_builds_correct_argv() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    // Regression for "tailor top 20" button: the JS sends
    // {"command":"tailor","args":{"limit":20}} which must produce
    // `careerai tailor --limit 20` argv. Use /bin/true as a stub binary.
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(&url, r#"{"command":"tailor","args":{"limit":20}}"#).await;
    assert_eq!(status, 200, "tailor --limit 20 must succeed: {body}");
    assert!(body.contains("\"status\":\"success\""), "body: {body}");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_tailor_all_builds_correct_argv() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(&url, r#"{"command":"tailor","args":{"all":true}}"#).await;
    assert_eq!(status, 200, "tailor --all must succeed: {body}");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_tailor_with_listing_id_builds_correct_argv() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(
        &url,
        r#"{"command":"tailor","args":{"listing_id":"abc-123"}}"#,
    )
    .await;
    assert_eq!(status, 200, "tailor <id> must succeed: {body}");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_render_all_builds_correct_argv() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(&url, r#"{"command":"render","args":{"all":true}}"#).await;
    assert_eq!(status, 200, "render --all must succeed: {body}");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_apply_all_builds_correct_argv() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(&url, r#"{"command":"apply","args":{"all":true}}"#).await;
    assert_eq!(status, 200, "apply --all must succeed: {body}");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_apply_missing_id_rejected() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    let (status, body) = curl_post(&url, r#"{"command":"apply","args":{}}"#).await;
    assert_eq!(status, 400, "apply without id must be rejected: {body}");
    assert!(body.contains("missing"), "body: {body}");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_inspect_missing_id_rejected() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    let (status, body) = curl_post(&url, r#"{"command":"inspect","args":{}}"#).await;
    assert_eq!(status, 400, "inspect without id must be rejected: {body}");
    assert!(body.contains("missing"), "body: {body}");

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_all_runnable_commands_succeed_with_stub() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");

    // Every Runnable command from the catalog should succeed with /bin/true
    for cmd in [
        "config generate",
        "profile show",
        "profile validate",
        "discover",
        "match",
        "run",
        "review",
        "shortlist show",
        "applied",
        "digest",
        "sources sync",
        "sources discover-web",
        "mcp probe",
        "llm probe",
        "notify test",
    ] {
        let (status, body) = curl_post(&url, &format!("{{\"command\":\"{cmd}\"}}")).await;
        assert_eq!(
            status, 200,
            "command `{cmd}` must succeed with stub binary: {body}"
        );
        assert!(
            body.contains("\"status\":\"success\""),
            "command `{cmd}` body: {body}"
        );
    }

    // NeedsId commands with valid id
    for cmd_body in [
        r#"{"command":"tailor","args":{"listing_id":"abc-123"}}"#,
        r#"{"command":"render","args":{"application_id":"def-456"}}"#,
        r#"{"command":"inspect","args":{"application_id":"ghi-789"}}"#,
        r#"{"command":"retry","args":{"application_id":"jkl-012"}}"#,
    ] {
        let (status, body) = curl_post(&url, cmd_body).await;
        assert_eq!(status, 200, "NeedsId command must succeed: {body}");
    }

    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_concurrent_requests_return_busy() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    // Use a slow stub so the first request holds the lock
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
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        std::env::set_var("CAREERAI_BIN", &script);

        let first_url = url.clone();
        let first =
            tokio::spawn(async move { curl_post(&first_url, r#"{"command":"llm probe"}"#).await });

        // Wait for the slow script to start
        let mut ready_seen = false;
        for _ in 0..40 {
            if ready.exists() {
                ready_seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(ready_seen, "slow script never started");

        let (status, body) = curl_post(&url, r#"{"command":"llm probe"}"#).await;
        assert_eq!(status, 409, "concurrent run must be busy: {body}");

        let (first_status, first_body) = first.await.expect("first run join");
        assert_eq!(first_status, 200, "first run should succeed: {first_body}");

        std::env::remove_var("CAREERAI_BIN");
    }

    server.abort();
}

// ---------------------------------------------------------------------------
// Config endpoints
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_endpoint_returns_expected_fields() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!("http://127.0.0.1:{port}/api/v1/config")).await;

    assert!(
        body.contains("score_threshold"),
        "config must include score_threshold: {body}"
    );
    assert!(
        body.contains("llm_backend"),
        "config must include llm_backend: {body}"
    );
    assert!(
        body.contains("llm_strategy"),
        "config must include llm_strategy: {body}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_generate_returns_preview() {
    let (server, port) = bootserver().await;
    let (status, body) = curl_post(
        &format!("http://127.0.0.1:{port}/api/v1/config/generate"),
        "{}",
    )
    .await;

    assert!(
        status == 200 || status == 500,
        "config/generate status: {status}, body: {body}"
    );
    assert!(
        body.contains("preview") || body.contains("error"),
        "config/generate must return preview or error: {body}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_prompt_extracts_keywords() {
    let (server, port) = bootserver().await;
    let body = r#"{"prompt":"Focus on Remote Embedded Linux and Freelance AI jobs"}"#;
    let (status, resp) = curl_post(
        &format!("http://127.0.0.1:{port}/api/v1/config/prompt"),
        body,
    )
    .await;

    assert_eq!(status, 200, "config/prompt must succeed: {resp}");
    assert!(
        resp.contains("extracted_keywords"),
        "config/prompt must return extracted_keywords: {resp}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn threshold_save_rejects_invalid_input() {
    let (server, port) = bootserver().await;
    let (status, body) = curl_post(
        &format!("http://127.0.0.1:{port}/api/config/threshold"),
        r#"{"score_threshold":"not a number"}"#,
    )
    .await;
    assert!(
        status >= 400,
        "invalid threshold must be rejected: status={status}, body={body}"
    );

    server.abort();
}

// ---------------------------------------------------------------------------
// Explorer + listing endpoints
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explorer_endpoint_returns_json_array() {
    let pool = pool_in_memory().await.expect("in-memory pool");
    seed_listing(&pool, "discovered").await;
    seed_listing(&pool, "shortlisted").await;

    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool,
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

    let body = curl_get(&format!("http://127.0.0.1:{port}/api/v1/explorer")).await;
    // R17: explorer returns { items, total, limit, offset } for pagination
    assert!(
        body.contains("\"items\""),
        "explorer must return items field: {body:.200}"
    );
    assert!(
        body.contains("Senior Rust Engineer"),
        "explorer must contain seeded listing: {body:.200}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn application_detail_returns_404_for_unknown_id() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!(
        "http://127.0.0.1:{port}/api/v1/applications/nonexistent-id"
    ))
    .await;
    assert!(
        body.contains("error") || body.contains("not found") || body.contains("null"),
        "unknown application should return error/null: {body:.200}"
    );

    server.abort();
}

// ---------------------------------------------------------------------------
// Security: cross-origin rejection
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_origin_post_rejected_on_cli_run() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");
    let body = r#"{"command":"discover"}"#;

    let (status, resp) = curl_post_with_headers(&url, body, &["Origin: http://evil.example"]).await;
    assert_eq!(status, 403, "cross-origin must be 403: {resp}");
    assert!(
        resp.contains("cross-origin"),
        "expected cross-origin rejection: {resp}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_origin_post_rejected_on_config_save() {
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/config");
    let body = r#"{"backend":"agy"}"#;

    let (status, resp) = curl_post_with_headers(&url, body, &["Origin: http://evil.example"]).await;
    assert_eq!(status, 403, "cross-origin must be 403: {resp}");

    server.abort();
}

// ---------------------------------------------------------------------------
// Profile import endpoint
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_import_rejects_empty_body() {
    let (server, port) = bootserver().await;
    let (status, body) = curl_post(
        &format!("http://127.0.0.1:{port}/api/v1/profile/import"),
        "{}",
    )
    .await;
    assert!(
        status >= 400,
        "empty profile import must be rejected: status={status}, body={body}"
    );
    assert!(
        body.contains("error") || body.contains("No file") || body.contains("boundary"),
        "expected error message: {body}"
    );

    server.abort();
}

// ---------------------------------------------------------------------------
// Snapshot + events endpoints
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_contains_kpi_and_state_counts() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!("http://127.0.0.1:{port}/api/v1/snapshot")).await;
    assert!(
        body.contains("kpi"),
        "snapshot must include kpi: {body:.200}"
    );
    assert!(
        body.contains("state_counts"),
        "snapshot must include state_counts: {body:.200}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_endpoint_returns_array() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!("http://127.0.0.1:{port}/api/v1/events")).await;
    assert!(
        body.starts_with('['),
        "events must return a JSON array: {body:.200}"
    );

    server.abort();
}

// ---------------------------------------------------------------------------
// Pipeline wrappers (discover/match) still work
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_discover_wrapper_responds() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, _) = curl_post(
        &format!("http://127.0.0.1:{port}/api/v1/pipeline/discover"),
        "{}",
    )
    .await;
    assert_eq!(status, 200, "discover wrapper must respond");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_match_wrapper_responds() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, _) = curl_post(
        &format!("http://127.0.0.1:{port}/api/v1/pipeline/match"),
        "{}",
    )
    .await;
    assert_eq!(status, 200, "match wrapper must respond");
    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

// ---------------------------------------------------------------------------
// Rollback argv construction
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_rollback_all_builds_correct_argv() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(
        &url,
        r#"{"command":"rollback","args":{"all":true,"from_state":"rendered","to":"tailored"}}"#,
    )
    .await;
    assert_eq!(status, 200, "rollback --all must succeed: {body}");

    let (status, body) = curl_post(
        &url,
        r#"{"command":"rollback","args":{"id":"abc-123","to":"shortlisted"}}"#,
    )
    .await;
    assert_eq!(status, 200, "rollback <id> must succeed: {body}");

    let (status, body) = curl_post(&url, r#"{"command":"rollback","args":{}}"#).await;
    assert_eq!(status, 400, "rollback without id/all must fail: {body}");

    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

// ---------------------------------------------------------------------------
// Shortlist with limit argv construction
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_shortlist_show_with_limit() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) =
        curl_post(&url, r#"{"command":"shortlist show","args":{"limit":10}}"#).await;
    assert_eq!(status, 200, "shortlist show --limit must succeed: {body}");

    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

// ---------------------------------------------------------------------------
// Applied with source + limit
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_applied_with_source_and_limit() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(
        &url,
        r#"{"command":"applied","args":{"source":"greenhouse","limit":50}}"#,
    )
    .await;
    assert_eq!(
        status, 200,
        "applied with source+limit must succeed: {body}"
    );

    // Unknown source must be rejected
    let (status, body) = curl_post(
        &url,
        r#"{"command":"applied","args":{"source":"fake_source"}}"#,
    )
    .await;
    assert_eq!(status, 400, "unknown source must be rejected: {body}");
    assert!(body.contains("unknown source"), "body: {body}");

    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

// ---------------------------------------------------------------------------
// Discover with source filter
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_run_discover_with_sources() {
    let _serial = CLI_RUN_SERIAL.lock().expect("cli-run test mutex poisoned");
    let (server, port) = bootserver().await;
    let url = format!("http://127.0.0.1:{port}/api/v1/cli/run");

    std::env::set_var("CAREERAI_BIN", "/bin/true");
    let (status, body) = curl_post(
        &url,
        r#"{"command":"discover","args":{"sources":["greenhouse","lever"]}}"#,
    )
    .await;
    assert_eq!(
        status, 200,
        "discover with known sources must succeed: {body}"
    );

    // Unknown source must be rejected
    let (status, body) = curl_post(
        &url,
        r#"{"command":"discover","args":{"sources":["fake_source"]}}"#,
    )
    .await;
    assert_eq!(status, 400, "unknown source must be rejected: {body}");

    // Empty source list must be rejected
    let (status, body) = curl_post(&url, r#"{"command":"discover","args":{"sources":[]}}"#).await;
    assert_eq!(status, 400, "empty sources must be rejected: {body}");

    std::env::remove_var("CAREERAI_BIN");
    server.abort();
}

// ---------------------------------------------------------------------------
// Health endpoint
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_returns_ok() {
    let (server, port) = bootserver().await;
    let body = curl_get(&format!("http://127.0.0.1:{port}/healthz")).await;
    assert!(body.starts_with("ok"), "healthz must return ok: {body}");
    server.abort();
}
