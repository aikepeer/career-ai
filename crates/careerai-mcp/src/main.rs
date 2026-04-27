//! `careerai-mcp` binary — speaks MCP over stdio.
//!
//! Logs go to stderr only; stdout is reserved for JSON-RPC framing.
//! The project root defaults to the current working directory and can
//! be overridden with the `CAREERAI_ROOT` environment variable.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use careerai_mcp::CareerAiServer;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let root = resolve_root().context("resolve project root")?;
    tracing::info!(root = %root.display(), "starting careerai-mcp on stdio");

    let server = CareerAiServer::new(root);
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!(error = %e, "mcp serve failed"))?;

    service.waiting().await?;
    Ok(())
}

fn resolve_root() -> Result<PathBuf> {
    if let Ok(env_root) = std::env::var("CAREERAI_ROOT") {
        return Ok(PathBuf::from(env_root));
    }
    std::env::current_dir().context("current_dir")
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
