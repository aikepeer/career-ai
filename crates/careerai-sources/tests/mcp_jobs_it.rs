//! Integration test for the MCP-jobs adapter.
//!
//! Strategy: the test binary re-execs itself with
//! `CAREERAI_FAKE_MCP=1` to act as a fake stdio MCP server. The
//! adapter spawns that fake server, runs `tools/list` + `tools/call`,
//! and the test asserts that the canned 2-listing response is
//! decoded into 2 `RawListing`s with the expected field mapping.
//!
//! Cargo runs this with `harness = false` so we own `main()`. The env
//! variable switch decides whether to (a) run the integration tests
//! as a normal client, or (b) speak MCP on stdio as a fake server.
//! `harness = false` is mandatory: the libtest harness would write a
//! banner to stdout before any code runs, corrupting JSON-RPC frames
//! the child must emit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async_trait_impl
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use careerai_core::config::{McpQueryConfig, McpSourceConfig, McpTransportConfig};
use careerai_sources::{McpJobsSource, Source};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
    ServerInfo,
};
use rmcp::service::ServiceExt;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Deserialize;

const FIXTURE_JSON: &str = include_str!("fixtures/jobs_response.json");

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    #[allow(dead_code)]
    keywords: String,
    #[serde(default)]
    #[allow(dead_code)]
    location: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    limit: Option<u32>,
}

#[derive(Clone)]
struct FakeJobsServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl FakeJobsServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Returns the embedded fixture as a single text content block.
    /// The adapter parses it with `parse_listings`.
    #[tool(description = "Search jobs by keyword + location.")]
    async fn search_jobs(
        &self,
        Parameters(_args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text(FIXTURE_JSON)]))
    }
}

#[tool_handler]
impl ServerHandler for FakeJobsServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_server_info(Implementation::new("fake-jobs-mcp", "0.0.1"))
            .with_instructions("Test fixture; returns a fixed 2-listing payload.")
    }
}

fn current_test_binary() -> PathBuf {
    std::env::current_exe().expect("current_exe")
}

#[tokio::main(flavor = "current_thread")]
async fn run_as_server() -> Result<(), Box<dyn std::error::Error>> {
    let stdio = (tokio::io::stdin(), tokio::io::stdout());
    let svc = FakeJobsServer::new().serve(stdio).await?;
    svc.waiting().await?;
    Ok(())
}

async fn discover_against_fake_server_returns_two_listings() {
    let bin = current_test_binary();
    let cfg = McpSourceConfig {
        name: "fake-mcp".into(),
        enabled: true,
        submit_enabled: false,
        cron: None,
        rate_per_minute: 0,
        mcp: McpTransportConfig {
            command: bin.to_string_lossy().into_owned(),
            args: vec![],
            env: std::iter::once(("CAREERAI_FAKE_MCP".to_string(), "1".to_string())).collect(),
            query: McpQueryConfig {
                keywords: "AI engineer".into(),
                location: Some("Worldwide".into()),
                limit: Some(5),
            },
        },
    };

    let src = McpJobsSource::new(cfg);
    let result = tokio::time::timeout(Duration::from_secs(20), src.discover())
        .await
        .expect("timeout waiting for discover()");
    let listings = result.expect("discover ok");
    assert_eq!(listings.len(), 2, "expected 2 listings, got {listings:?}");

    let l0 = &listings[0];
    assert_eq!(l0.source, "fake-mcp");
    assert_eq!(l0.title, "Senior AI Engineer");
    assert_eq!(l0.company, "Acme Robotics");
    assert_eq!(l0.location.as_deref(), Some("Remote (India)"));
    assert_eq!(l0.url, "https://example.com/jobs/ai-1");
    assert_eq!(l0.external_id, "fixture-1");
    assert!(
        l0.description.contains("LLM"),
        "description not mapped: {:?}",
        l0.description,
    );

    let l1 = &listings[1];
    assert_eq!(l1.title, "Embedded ML Engineer");
    assert_eq!(l1.company, "Beta Edge");
    assert_eq!(l1.location.as_deref(), Some("Bangalore-remote"));
    assert_eq!(l1.url, "https://example.com/jobs/em-2");
    assert_eq!(l1.external_id, "fixture-2");
}

fn registry_routes_mcp_source_with_static_name() {
    let cfg = McpSourceConfig {
        name: "registry-test-mcp".into(),
        enabled: true,
        submit_enabled: false,
        cron: None,
        rate_per_minute: 0,
        mcp: McpTransportConfig {
            command: "/bin/false".into(),
            args: vec![],
            env: std::collections::HashMap::default(),
            query: McpQueryConfig::default(),
        },
    };
    let src = McpJobsSource::new(cfg);
    assert_eq!(src.name(), "registry-test-mcp");
    let boxed: Box<dyn Source> = Box::new(src);
    assert_eq!(boxed.name(), "registry-test-mcp");
}

#[tokio::main(flavor = "current_thread")]
async fn run_tests() -> Result<(), Box<dyn std::error::Error>> {
    discover_against_fake_server_returns_two_listings().await;
    registry_routes_mcp_source_with_static_name();
    println!("all mcp_jobs_it tests passed");
    Ok(())
}

fn main() -> ExitCode {
    if std::env::var_os("CAREERAI_FAKE_MCP").is_some() {
        // Child mode: speak MCP on stdio. Stdout is reserved for
        // JSON-RPC; never write anything else to it.
        match run_as_server() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("fake-jobs-mcp server error: {e:#}");
                ExitCode::FAILURE
            }
        }
    } else {
        if std::env::args().any(|a| a == "--list") {
            println!("mcp_jobs_integration: test");
            return ExitCode::SUCCESS;
        }
        match run_tests() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("test driver error: {e:#}");
                ExitCode::FAILURE
            }
        }
    }
}
