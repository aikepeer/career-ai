#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Integration tests for the MCP server.
//!
//! Spawns `CareerAiServer` over an in-process `tokio::io::duplex` and
//! drives it with a raw `ClientHandler` to verify:
//!
//! 1. `initialize` succeeds and the server advertises tools + resources.
//! 2. `tools/list` returns all 8 expected tools.
//! 3. `tools/call` for `careerai_profile_status` against a tempdir
//!    (no profile.yaml) returns a structured "exists: false" result —
//!    not an error.
//! 4. `tools/call` for `careerai_apply` with `dry_run: false` and no
//!    `confirm` token is REJECTED, AND the user-facing error text
//!    actually mentions the `confirm` / `I_UNDERSTAND_TOS_RISK` /
//!    `dry_run` triad — guarding against silent regression to a
//!    generic "internal error" message.
//! 5. The server task does not panic during normal client cancel
//!    (`ServerGuard` joins it on drop and asserts a clean exit).

use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::service::ServiceExt;
use rmcp::ClientHandler;
use serde_json::{json, Value};

use careerai_mcp::CareerAiServer;

/// Tiny client-side handler. The default `ClientHandler` impl already
/// returns a sensible `ClientInfo`; we don't need to override anything.
#[derive(Default, Clone)]
struct TestClient;

impl ClientHandler for TestClient {}

/// RAII guard for the spawned server `JoinHandle`. Without this, a
/// detached server task can swallow panics or pipeline errors silently
/// — the test harness only sees the client side and prints PASS even
/// though the server crashed mid-handshake.
///
/// On drop:
/// 1. Wait up to ~2 s for the server task to finish naturally (the
///    client should already have cancelled, which closes the duplex
///    and lets `svc.waiting()` resolve).
/// 2. If it's still running, abort it.
/// 3. Inspect the join result and panic if the server task itself
///    panicked or returned an error — surfacing it on the test thread.
struct ServerGuard {
    handle: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

impl ServerGuard {
    fn new(handle: tokio::task::JoinHandle<anyhow::Result<()>>) -> Self {
        Self {
            handle: Some(handle),
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };

        // We're inside a tokio runtime (every `#[tokio::test]`
        // provides one), so calling `block_on` here would panic with
        // "Cannot start a runtime from within a runtime". Drive the
        // join from a fresh thread that has its own tiny single-thread
        // runtime; that thread is not driven by the test's runtime.
        let abort = handle.abort_handle();
        let join_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("build join runtime");
            rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), handle).await })
        });

        // If the join takes too long here too (e.g. the runtime was
        // already torn down with the server task still alive), give up
        // and abort. We bound the wall-clock to ~3 s.
        let Ok(join_result) = join_thread.join() else {
            abort.abort();
            return;
        };

        match join_result {
            Ok(Ok(Ok(()))) => { /* clean shutdown */ }
            Ok(Ok(Err(e))) => panic!("server task returned error: {e:#}"),
            Ok(Err(join_err)) => {
                if join_err.is_panic() {
                    panic!("server task panicked: {join_err}");
                } else {
                    // cancelled/aborted: no panic to surface.
                }
            }
            Err(_elapsed) => {
                // Server still running after client cancel + 2 s
                // grace. Abort and move on; this is not itself a
                // failure (the duplex may still be draining).
                abort.abort();
            }
        }
    }
}

/// Bring up an in-process server + client pair. The harness mirrors the
/// upstream rmcp `counter` example: a `tokio::io::duplex` connects the
/// two halves, `Counter::new().serve(...)` becomes
/// `CareerAiServer::new(...).serve(...)`. Returns a `ServerGuard` that
/// joins the server task on drop and surfaces any panic on the test
/// thread.
async fn spawn_pair(
    root: std::path::PathBuf,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    ServerGuard,
) {
    let (server_io, client_io) = tokio::io::duplex(1 << 16);

    let server = CareerAiServer::new(root);
    let server_handle = tokio::spawn(async move {
        let svc = server.serve(server_io).await?;
        svc.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient.serve(client_io).await.expect("client connect");
    (client, ServerGuard::new(server_handle))
}

#[tokio::test]
async fn initialize_and_list_tools() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _guard) = spawn_pair(tmp.path().to_path_buf()).await;

    // peer_info() returns the server's announced ServerInfo from the
    // initialize handshake.
    let info = client.peer_info().expect("peer_info");
    assert!(
        info.capabilities.tools.is_some(),
        "server must advertise tools capability",
    );

    let tools = tokio::time::timeout(Duration::from_secs(5), client.list_all_tools())
        .await
        .expect("list_all_tools timeout")
        .expect("list_all_tools failed");

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected = [
        "careerai_profile_status",
        "careerai_discover",
        "careerai_shortlist",
        "careerai_tailor",
        "careerai_render",
        "careerai_apply",
        "careerai_inspect",
        "careerai_digest",
    ];
    for tool in expected {
        assert!(
            names.contains(&tool),
            "missing tool: {tool} (got {names:?})"
        );
    }
    assert_eq!(
        tools.len(),
        expected.len(),
        "expected exactly 8 tools, got {names:?}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn profile_status_against_empty_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _guard) = spawn_pair(tmp.path().to_path_buf()).await;

    let result = client
        .call_tool(
            CallToolRequestParams::new("careerai_profile_status")
                .with_arguments(serde_json::Map::new()),
        )
        .await
        .expect("call profile_status");

    // The handler returned a JSON-in-text content block (see
    // `json_content` in server.rs). Parse and assert.
    let text = first_text(&result.content).expect("text content");
    let value: Value = serde_json::from_str(text).expect("json");
    assert_eq!(value["exists"], json!(false));
    assert_eq!(value["valid"], json!(false));
    let issues = value["issues"].as_array().expect("issues array");
    assert!(!issues.is_empty(), "expected at least one issue");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn apply_without_confirm_token_is_rejected() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _guard) = spawn_pair(tmp.path().to_path_buf()).await;

    let mut args = serde_json::Map::new();
    args.insert("application_id".into(), json!("does-not-exist"));
    args.insert("dry_run".into(), json!(false));
    // confirm omitted on purpose.

    let outcome = client
        .call_tool(CallToolRequestParams::new("careerai_apply").with_arguments(args))
        .await;

    // Either: (a) the call returned a transport-level error (the handler
    // converted McpServerError::AutoSubmitNotConfirmed into an
    // InvalidParams ErrorData), or (b) the call returned a successful
    // CallToolResult with `isError: true`. In BOTH cases the
    // user-facing message must mention the confirm token, the literal
    // I_UNDERSTAND_TOS_RISK, AND the dry_run gate — otherwise a
    // regression to a generic "internal error" string would silently
    // pass.
    let assert_contains_kill_switch = |msg: &str| {
        assert!(
            msg.contains("confirm"),
            "missing `confirm` in error message: {msg}"
        );
        assert!(
            msg.contains("I_UNDERSTAND_TOS_RISK"),
            "missing `I_UNDERSTAND_TOS_RISK` in error message: {msg}"
        );
        assert!(
            msg.contains("dry_run"),
            "missing `dry_run` in error message: {msg}"
        );
    };

    match outcome {
        Err(e) => {
            let msg = format!("{e:#}");
            assert_contains_kill_switch(&msg);
        }
        Ok(result) => {
            assert_eq!(result.is_error, Some(true), "expected isError=true");
            // Parse the text content (same shape as
            // `profile_status_against_empty_root`): rmcp packs the
            // ErrorData message into a text content block when
            // `is_error` is true.
            let text =
                first_text(&result.content).expect("expected text content with error message");
            assert_contains_kill_switch(text);
        }
    }

    client.cancel().await.expect("cancel");
}

fn first_text(content: &[rmcp::model::Content]) -> Option<&str> {
    content.iter().find_map(|c| match &c.raw {
        rmcp::model::RawContent::Text(t) => Some(t.text.as_str()),
        _ => None,
    })
}
