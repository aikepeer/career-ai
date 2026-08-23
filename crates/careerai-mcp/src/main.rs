//! `careerai-mcp` binary — speaks MCP over stdio.
//!
//! Logs go to stderr only; stdout is reserved for JSON-RPC framing.
//! The project root defaults to the current working directory and can
//! be overridden with the `CAREERAI_ROOT` environment variable.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use careerai_mcp::CareerAiServer;

/// MCP (Model Context Protocol) server exposing the career-ai pipeline
/// as tool calls. Speaks JSON-RPC over stdio; logs go to stderr only.
///
/// Set `CAREERAI_ROOT` to override the project root (defaults to the
/// current working directory). Set `CAREERAI_LOG` to override the
/// tracing filter (defaults to `info,careerai_mcp=debug`).
#[derive(Parser, Debug)]
#[command(name = "careerai-mcp", version, about, long_about = None)]
struct Cli {
    /// Override the project root. Equivalent to setting CAREERAI_ROOT.
    #[arg(long, value_name = "DIR", env = "CAREERAI_ROOT")]
    root: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing();

    let root = match cli.root {
        Some(p) => p,
        None => careerai_core::paths::resolve_root_env(),
    };
    tracing::info!(root = %root.display(), "starting careerai-mcp on stdio");

    let server = CareerAiServer::new(root);
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!(error = %e, "mcp serve failed"))?;

    service.waiting().await?;
    Ok(())
}

fn init_tracing() {
    // Default to `info`. Filter overrideable via `CAREERAI_LOG`.
    let filter = EnvFilter::try_from_env("CAREERAI_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,careerai_mcp=debug"));

    // CRITICAL: write to stderr only — stdout is the MCP transport.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}
