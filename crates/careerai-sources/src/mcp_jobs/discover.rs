//! Async discovery pipeline: MCP connect, tool selection, call, response
//! parsing, and the `probe` CLI helper.

use std::time::Duration;

use careerai_core::config::{McpSourceConfig, McpTransportConfig};
use rmcp::model::{CallToolRequestParams, Content as McpContent, RawContent};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use serde_json::{Map, Value};
use tokio::process::Command;
use tracing::{debug, info};

use crate::base::{RawListing, SourceError};

use super::parser::parse_listings;
use super::source::{ProbeClient, TOOL_NAME_ALIASES};

pub(crate) async fn run_call(
    src: &super::source::McpJobsSource,
) -> Result<Vec<RawListing>, SourceError> {
    let svc = connect(src).await?;

    // Run the call inside a closure so we can guarantee `svc.cancel()`
    // fires whether the inner work succeeds or fails. Returning early
    // with `?` would otherwise drop `svc` on the floor and rely solely
    // on `kill_on_drop` to reap the child — cleaner to send the MCP
    // shutdown frame first.
    let inner = call_and_parse(src, &svc).await;

    // Best-effort shutdown; ignore errors so a misbehaving server
    // can't poison the cron tick. `kill_on_drop` (set in
    // `build_command`) is the SIGKILL backstop when this fails.
    let _ = svc.cancel().await;

    inner
}

async fn connect(
    src: &super::source::McpJobsSource,
) -> Result<RunningService<RoleClient, ProbeClient>, SourceError> {
    let cmd = build_command(&src.cfg().mcp);
    let child = TokioChildProcess::new(cmd).map_err(|e| {
        SourceError::Parse(format!("spawn mcp server `{}`: {e}", src.cfg().mcp.command))
    })?;
    let svc = ProbeClient
        .serve(child)
        .await
        .map_err(|e| SourceError::Parse(format!("mcp initialize failed: {e}")))?;
    Ok(svc)
}

async fn call_and_parse(
    src: &super::source::McpJobsSource,
    svc: &RunningService<RoleClient, ProbeClient>,
) -> Result<Vec<RawListing>, SourceError> {
    let tool_name = pick_tool(src, svc).await?;
    let args = build_tool_args(&src.cfg().mcp.query);

    let result = svc
        .call_tool(CallToolRequestParams::new(tool_name.clone()).with_arguments(args))
        .await
        .map_err(|e| SourceError::Parse(format!("tools/call `{tool_name}` failed: {e}")))?;

    if result.is_error.unwrap_or(false) {
        let text = first_text_content(&result.content);
        return Err(SourceError::Parse(format!(
            "mcp tool `{tool_name}` returned isError: {}",
            text.unwrap_or_else(|| "(no text content)".to_string())
        )));
    }

    let raw_json = first_text_content(&result.content).ok_or_else(|| {
        SourceError::Parse(format!(
            "mcp tool `{tool_name}` returned no text content blocks"
        ))
    })?;

    // Surface parse errors instead of swallowing them. A community MCP
    // server that suddenly switches schema would otherwise show up as
    // "0 listings" in the cron log with no indication that the daemon
    // is silently broken — the operator only notices days later.
    let listings = parse_listings(&raw_json, src.name_static).map_err(SourceError::Parse)?;
    info!(
        source = %src.name_static,
        count = listings.len(),
        "mcp adapter discovered listings",
    );
    Ok(listings)
}

/// Pick the first tool from `tools/list` whose name matches one of
/// [`TOOL_NAME_ALIASES`]. Returns the chosen tool name.
async fn pick_tool(
    src: &super::source::McpJobsSource,
    svc: &RunningService<RoleClient, ProbeClient>,
) -> Result<String, SourceError> {
    let tools = svc
        .list_all_tools()
        .await
        .map_err(|e| SourceError::Parse(format!("tools/list failed: {e}")))?;
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    debug!(source = %src.name_static, tools = ?names, "mcp tools/list");
    for alias in TOOL_NAME_ALIASES {
        if let Some(found) = names.iter().find(|n| n.as_str() == *alias) {
            info!(
                source = %src.name_static,
                tool = %found,
                "mcp adapter selected tool",
            );
            return Ok(found.clone());
        }
    }
    Err(SourceError::Parse(format!(
        "no job-search tool advertised by `{}` (looked for one of {:?}, got {:?})",
        src.name_static, TOOL_NAME_ALIASES, names
    )))
}

/// Extract the first text-content block from a `tools/call` result, if
/// any. Most servers emit `{ "content": [{ "type": "text", "text": "..." }] }`.
fn first_text_content(content: &[McpContent]) -> Option<String> {
    content.iter().find_map(|c| match &c.raw {
        RawContent::Text(t) => Some(t.text.clone()),
        _ => None,
    })
}

pub(crate) fn build_command(cfg: &McpTransportConfig) -> Command {
    let mut cmd = Command::new(&cfg.command);
    cmd.args(&cfg.args);
    for (k, v) in &cfg.env {
        cmd.env(k, expand_env(v));
    }
    // If the cron tick times out (or we hit any early-error path before
    // `svc.cancel().await` fires), the `RunningService` is dropped but
    // the spawned MCP server child does not necessarily die with it.
    // `kill_on_drop(true)` makes tokio SIGKILL the child when the
    // `TokioChildProcess` handle is dropped, so a hung `uvx` / `npx` /
    // `docker` cannot accumulate one-orphan-per-tick.
    cmd.kill_on_drop(true);
    cmd
}

/// Expand `${NAME}` references against the parent environment. Only
/// the bare `${NAME}` form is supported (no `${NAME:-default}`); a
/// missing var collapses to an empty string. This is intentionally
/// minimal — full shell expansion would invite injection bugs.
pub(crate) fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        if let Some(end) = rest.find('}') {
            let var = &rest[..end];
            let value = std::env::var(var).unwrap_or_default();
            out.push_str(&value);
            rest = &rest[end + 1..];
        } else {
            // Unterminated; treat the rest as literal.
            out.push_str("${");
            out.push_str(rest);
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn build_tool_args(q: &careerai_core::config::McpQueryConfig) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("keywords".into(), Value::String(q.keywords.clone()));
    if let Some(loc) = &q.location {
        args.insert("location".into(), Value::String(loc.clone()));
    }
    if let Some(limit) = q.limit {
        args.insert("limit".into(), Value::Number(limit.into()));
    }
    args
}

/// Wire helper used by the `careerai mcp probe` CLI subcommand. Spawns
/// the configured server, runs `initialize` + `tools/list`, and
/// reports which (if any) tool name matched a known job-search alias.
/// Wrapped in a 5s timeout per spec.
pub async fn probe(cfg: &McpSourceConfig) -> Result<ProbeReport, SourceError> {
    let result = tokio::time::timeout(Duration::from_secs(5), async move {
        let client = ProbeClient;
        let cmd = build_command(&cfg.mcp);
        let child = TokioChildProcess::new(cmd).map_err(|e| {
            SourceError::Parse(format!("spawn mcp server `{}`: {e}", cfg.mcp.command))
        })?;
        let svc = client
            .serve(child)
            .await
            .map_err(|e| SourceError::Parse(format!("mcp initialize failed: {e}")))?;
        let tools = svc
            .list_all_tools()
            .await
            .map_err(|e| SourceError::Parse(format!("tools/list failed: {e}")))?;
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        let matched = TOOL_NAME_ALIASES
            .iter()
            .find_map(|a| names.iter().find(|n| n.as_str() == *a).cloned());
        let _ = svc.cancel().await;
        Ok::<ProbeReport, SourceError>(ProbeReport {
            source_name: cfg.name.clone(),
            tool_count: names.len(),
            tools: names,
            matched_tool: matched,
        })
    })
    .await
    .map_err(|_| SourceError::Parse(format!("probe `{}` timed out after 5s", cfg.name)))??;
    Ok(result)
}

#[derive(Debug)]
pub struct ProbeReport {
    pub source_name: String,
    pub tool_count: usize,
    pub tools: Vec<String>,
    pub matched_tool: Option<String>,
}
