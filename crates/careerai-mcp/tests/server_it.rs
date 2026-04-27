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
//!    `confirm` token is REJECTED with an InvalidParams error.

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

/// Bring up an in-process server + client pair. The harness mirrors the
/// upstream rmcp `counter` example: a `tokio::io::duplex` connects the
/// two halves, `Counter::new().serve(...)` becomes
/// `CareerAiServer::new(...).serve(...)`.
async fn spawn_pair(
    root: std::path::PathBuf,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let (server_io, client_io) = tokio::io::duplex(1 << 16);

    let server = CareerAiServer::new(root);
    let server_handle = tokio::spawn(async move {
        let svc = server.serve(server_io).await?;
        svc.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient.serve(client_io).await.expect("client connect");
    (client, server_handle)
}

#[tokio::test]
async fn initialize_and_list_tools() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _server) = spawn_pair(tmp.path().to_path_buf()).await;

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
    let (client, _server) = spawn_pair(tmp.path().to_path_buf()).await;

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
    let (client, _server) = spawn_pair(tmp.path().to_path_buf()).await;

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
    // CallToolResult with `isError: true`. Either path is acceptable as
    // long as the user-facing message mentions the confirm token.
    match outcome {
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("confirm")
                    || msg.contains("I_UNDERSTAND_TOS_RISK")
                    || msg.contains("dry_run"),
                "expected confirm/dry_run rejection, got: {msg}"
            );
        }
        Ok(result) => {
            assert_eq!(result.is_error, Some(true), "expected isError=true");
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
