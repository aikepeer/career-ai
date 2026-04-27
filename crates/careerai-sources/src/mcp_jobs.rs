//! MCP-server discovery adapter.
//!
//! Reads jobs from a remote stdio Model-Context-Protocol server. The
//! server is spawned per `discover()` call; we negotiate `initialize`,
//! list its tools, pick the first job-search-shaped tool by name, and
//! invoke it with the configured query. The response is decoded into
//! `RawListing`s using a permissive field-mapping that tolerates the
//! schema drift between community MCP servers (e.g. RapidAPI's
//! linkedin-jobs vs. mcp-linkedin vs. generic ATS proxies).
//!
//! The adapter NEVER runs the local careerai-mcp server — that one is
//! consumed by Claude. This adapter consumes OTHER people's MCP
//! servers as discovery feeds for the daemon.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use careerai_core::config::{McpSourceConfig, McpTransportConfig};
use chrono::{DateTime, Utc};
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use rmcp::ClientHandler;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::base::{RawListing, Source, SourceError};

/// Tool-name aliases tried in order. The first matching tool advertised
/// by the server is used.
const TOOL_NAME_ALIASES: &[&str] = &[
    "search_jobs",
    "discover_jobs",
    "find_jobs",
    "list_jobs",
    "jobs.search",
];

/// Wall-clock cap for `initialize` + `tools/list` + the tool call.
/// Community MCP servers occasionally hang on slow upstream APIs; a
/// hard ceiling keeps a misbehaving source from blocking the cron tick.
const CALL_TIMEOUT: Duration = Duration::from_secs(45);

/// Default empty client handler. The default impl already advertises
/// sensible `ClientInfo`; we don't need elicitation, sampling, or
/// roots.
#[derive(Default, Clone, Debug)]
struct ProbeClient;

impl ClientHandler for ProbeClient {}

/// Per-instance read-side rate limiter. We deliberately do NOT pull
/// `careerai-submit::RateLimiter` here: that crate is downstream of
/// `careerai-sources` (sources never depends on submit, see
/// `CLAUDE.md`'s crate-boundary table) and its day-cap / quiet-hours
/// machinery is sized for write-side traffic. Read-side discovery
/// gets a simple `governor` token-bucket sized in calls-per-minute,
/// which is enough to keep us inside community RapidAPI free-tier
/// limits without inverting the dep graph.
type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

#[derive(Debug)]
pub struct McpJobsSource {
    cfg: McpSourceConfig,
    /// `&'static str` produced via `Box::leak`. Each `McpJobsSource`
    /// instance leaks at most one short string for the lifetime of the
    /// process; sources are constructed once per `build_sources()` and
    /// the daemon is single-process, so the leak is bounded.
    name_static: &'static str,
    rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl McpJobsSource {
    /// Construct a new adapter. `cfg.name` is leaked into a
    /// `&'static str` so the existing `Source::name(&self) -> &'static
    /// str` contract holds without allocating on every call.
    #[must_use]
    pub fn new(cfg: McpSourceConfig) -> Self {
        let name_static: &'static str = Box::leak(cfg.name.clone().into_boxed_str());
        let rate_limiter = NonZeroU32::new(cfg.rate_per_minute)
            .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
        Self {
            cfg,
            name_static,
            rate_limiter,
        }
    }

    /// Spawn the configured stdio process and complete the MCP
    /// handshake. Returns a connected client service.
    async fn connect(&self) -> Result<RunningService<RoleClient, ProbeClient>, SourceError> {
        let cmd = build_command(&self.cfg.mcp);
        let child = TokioChildProcess::new(cmd).map_err(|e| {
            SourceError::Parse(format!("spawn mcp server `{}`: {e}", self.cfg.mcp.command))
        })?;
        let svc = ProbeClient
            .serve(child)
            .await
            .map_err(|e| SourceError::Parse(format!("mcp initialize failed: {e}")))?;
        Ok(svc)
    }

    /// Pick the first tool from `tools/list` whose name matches one of
    /// [`TOOL_NAME_ALIASES`]. Returns the chosen tool name.
    async fn pick_tool(
        &self,
        svc: &RunningService<RoleClient, ProbeClient>,
    ) -> Result<String, SourceError> {
        let tools = svc
            .list_all_tools()
            .await
            .map_err(|e| SourceError::Parse(format!("tools/list failed: {e}")))?;
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        debug!(source = %self.name_static, tools = ?names, "mcp tools/list");
        for alias in TOOL_NAME_ALIASES {
            if let Some(found) = names.iter().find(|n| n.as_str() == *alias) {
                info!(
                    source = %self.name_static,
                    tool = %found,
                    "mcp adapter selected tool",
                );
                return Ok(found.clone());
            }
        }
        Err(SourceError::Parse(format!(
            "no job-search tool advertised by `{}` (looked for one of {:?}, got {:?})",
            self.name_static, TOOL_NAME_ALIASES, names
        )))
    }
}

#[async_trait]
impl Source for McpJobsSource {
    fn name(&self) -> &'static str {
        self.name_static
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        if let Some(rl) = &self.rate_limiter {
            // Read-side permit. `until_ready` waits if the bucket is
            // empty, but never blocks longer than `1/rate_per_minute`
            // — bounded by the per-minute quota — so the daemon tick
            // is not held hostage by a misconfigured cap.
            rl.until_ready().await;
        }

        let result = tokio::time::timeout(CALL_TIMEOUT, run_call(self))
            .await
            .map_err(|_| {
                SourceError::Parse(format!(
                    "mcp source `{}` timed out after {}s",
                    self.name_static,
                    CALL_TIMEOUT.as_secs(),
                ))
            })??;
        Ok(result)
    }
}

async fn run_call(src: &McpJobsSource) -> Result<Vec<RawListing>, SourceError> {
    let svc = src.connect().await?;
    let tool_name = src.pick_tool(&svc).await?;
    let args = build_tool_args(&src.cfg.mcp.query);

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

    let raw_json = first_text_content(&result.content).unwrap_or_default();
    let listings = parse_listings(&raw_json, src.name_static).unwrap_or_else(|err| {
        warn!(
            source = %src.name_static,
            error = %err,
            "mcp response could not be parsed as JSON listings; returning empty",
        );
        Vec::new()
    });
    info!(
        source = %src.name_static,
        count = listings.len(),
        "mcp adapter discovered listings",
    );

    // Best-effort cancel; ignore the result so a misbehaving server
    // can't poison the cron tick.
    let _ = svc.cancel().await;
    Ok(listings)
}

/// Extract the first text-content block from a `tools/call` result, if
/// any. Most servers emit `{ "content": [{ "type": "text", "text": "..." }] }`.
fn first_text_content(content: &[rmcp::model::Content]) -> Option<String> {
    content.iter().find_map(|c| match &c.raw {
        rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
        _ => None,
    })
}

/// Build the `tokio::process::Command` for the configured stdio MCP
/// server. `${VAR}` placeholders in `env` values are expanded against
/// the parent process's environment; missing vars expand to empty
/// strings (the server is responsible for surfacing the missing-key
/// error in its own logs).
fn build_command(cfg: &McpTransportConfig) -> Command {
    let mut cmd = Command::new(&cfg.command);
    cmd.args(&cfg.args);
    for (k, v) in &cfg.env {
        cmd.env(k, expand_env(v));
    }
    cmd
}

/// Expand `${NAME}` references against the parent environment. Only
/// the bare `${NAME}` form is supported (no `${NAME:-default}`); a
/// missing var collapses to an empty string. This is intentionally
/// minimal — full shell expansion would invite injection bugs.
fn expand_env(input: &str) -> String {
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

/// Parse a JSON-text response body into `RawListing`s.
///
/// Community MCP servers return one of:
/// - a top-level array (`[ {...}, {...} ]`)
/// - an object wrapping the array under `results` / `jobs` / `data`.
///
/// Inside each entry we try multiple field aliases per logical column
/// and keep the first non-empty hit. Unknown fields are preserved as
/// the `raw_json` payload for debugging.
pub(crate) fn parse_listings(body: &str, source_name: &str) -> Result<Vec<RawListing>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    let arr = extract_array(&value).ok_or_else(|| {
        format!(
            "expected JSON array or {{ results | jobs | data: [...] }}; got {}",
            value.as_object().map_or_else(
                || "scalar".into(),
                |o| format!("object with keys {:?}", o.keys().collect::<Vec<_>>()),
            )
        )
    })?;
    Ok(arr
        .iter()
        .filter_map(|item| map_listing(item, source_name))
        .collect())
}

/// Walk the common envelopes (`results`, `jobs`, `data`) to find the
/// listings array.
fn extract_array(value: &Value) -> Option<&Vec<Value>> {
    if let Some(arr) = value.as_array() {
        return Some(arr);
    }
    let obj = value.as_object()?;
    for key in ["results", "jobs", "data", "listings", "items"] {
        if let Some(Value::Array(a)) = obj.get(key) {
            return Some(a);
        }
    }
    None
}

fn map_listing(item: &Value, source_name: &str) -> Option<RawListing> {
    let title = pick_string(item, &["title", "job_title", "position"]).unwrap_or_default();
    let company = pick_string(item, &["company", "company_name", "employer"]).unwrap_or_default();
    let url = pick_string(item, &["url", "apply_url", "link"]).unwrap_or_default();
    if title.is_empty() && company.is_empty() && url.is_empty() {
        // Nothing useful — skip the row rather than emit a ghost.
        return None;
    }
    let location = pick_string(item, &["location", "place", "city"]);
    let description = pick_string(item, &["description", "summary", "snippet"]).unwrap_or_default();
    let external_id = pick_string(item, &["id", "job_id", "external_id"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| derive_external_id(source_name, &url, &title, &company));
    let _posted_at_unused: Option<DateTime<Utc>> =
        pick_string(item, &["posted_at", "date", "posted_date"])
            .as_deref()
            .and_then(parse_posted_at);
    Some(RawListing {
        source: source_name.to_string(),
        external_id,
        title,
        company,
        location,
        url,
        description,
        raw_json: Some(item.to_string()),
    })
}

/// Try each candidate key. If the value is a string, return it; if it
/// is a non-string scalar, stringify it. Empty strings count as
/// "missing" so the next alias gets a chance.
fn pick_string(item: &Value, keys: &[&str]) -> Option<String> {
    let obj = item.as_object()?;
    for key in keys {
        match obj.get(*key) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            Some(Value::Bool(b)) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}

/// SHA256 over `(source_name, url, title, company)` truncated to 16
/// hex chars. Used when the server doesn't expose a stable ID; the
/// downstream upsert key `(source, external_id)` then dedupes runs as
/// long as the server returns the same `(url, title, company)`.
fn derive_external_id(source_name: &str, url: &str, title: &str, company: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(url.as_bytes());
    hasher.update(b"\0");
    hasher.update(title.as_bytes());
    hasher.update(b"\0");
    hasher.update(company.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

/// Best-effort RFC3339 parse, with a tolerant fallback that strips a
/// trailing `Z` if `parse_from_rfc3339` rejects it.
fn parse_posted_at(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    None
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_rapidapi_shape() {
        // Mimic RapidAPI's linkedin-jobs response: array of objects
        // with `job_title` / `company_name` / `apply_url` / `posted_at`.
        let body = r#"[
            {
                "job_title": "AI Engineer",
                "company_name": "Acme",
                "location": "Remote",
                "apply_url": "https://example.com/jobs/1",
                "summary": "Build LLM apps",
                "job_id": "rapid-1",
                "posted_at": "2026-04-26T10:00:00Z"
            }
        ]"#;
        let listings = parse_listings(body, "linkedin-jobs-mcp").expect("parse");
        assert_eq!(listings.len(), 1);
        let l = &listings[0];
        assert_eq!(l.title, "AI Engineer");
        assert_eq!(l.company, "Acme");
        assert_eq!(l.url, "https://example.com/jobs/1");
        assert_eq!(l.location.as_deref(), Some("Remote"));
        assert_eq!(l.description, "Build LLM apps");
        assert_eq!(l.external_id, "rapid-1");
        assert_eq!(l.source, "linkedin-jobs-mcp");
    }

    #[test]
    fn parses_mcp_linkedin_shape() {
        // mcp-linkedin (Adhikary97 et al) returns objects under a
        // `jobs` envelope and uses `title` / `company` / `link`.
        let body = r#"{
            "jobs": [
                {
                    "title": "ML Researcher",
                    "company": "Beta",
                    "place": "Bangalore-remote",
                    "link": "https://example.com/jobs/2",
                    "description": "<p>Train models.</p>",
                    "id": "ml-2"
                },
                {
                    "title": "Robotics Engineer",
                    "employer": "Gamma",
                    "city": "Delhi NCR",
                    "url": "https://example.com/jobs/3",
                    "snippet": "ROS2",
                    "external_id": "rb-3"
                }
            ]
        }"#;
        let listings = parse_listings(body, "mcp-linkedin").expect("parse");
        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].title, "ML Researcher");
        assert_eq!(listings[0].company, "Beta");
        assert_eq!(listings[0].location.as_deref(), Some("Bangalore-remote"));
        assert_eq!(listings[1].company, "Gamma");
        assert_eq!(listings[1].location.as_deref(), Some("Delhi NCR"));
        assert_eq!(listings[1].url, "https://example.com/jobs/3");
        assert_eq!(listings[1].description, "ROS2");
    }

    #[test]
    fn parses_generic_shape_with_results_envelope() {
        let body = r#"{
            "results": [
                {
                    "position": "LLM Platform Engineer",
                    "employer": "Delta",
                    "url": "https://example.com/jobs/4"
                }
            ]
        }"#;
        let listings = parse_listings(body, "generic-mcp").expect("parse");
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].title, "LLM Platform Engineer");
        assert_eq!(listings[0].company, "Delta");
        // No id field anywhere → SHA256-derived external_id.
        assert_eq!(listings[0].external_id.len(), 16);
        assert!(listings[0]
            .external_id
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn missing_keys_yield_none_or_empty_no_panic() {
        let body = r#"[
            { "title": "Only a title" },
            { "url": "https://example.com/jobs/empty", "company": "Eps" },
            {}
        ]"#;
        let listings = parse_listings(body, "test").expect("parse");
        // The empty object has no usable fields; it should be skipped.
        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].title, "Only a title");
        assert_eq!(listings[0].company, "");
        assert_eq!(listings[0].location, None);
        assert_eq!(listings[1].url, "https://example.com/jobs/empty");
    }

    #[test]
    fn external_id_is_stable_for_same_inputs() {
        let a = derive_external_id("src", "u", "t", "c");
        let b = derive_external_id("src", "u", "t", "c");
        assert_eq!(a, b);
        let c = derive_external_id("src", "u", "t", "different");
        assert_ne!(a, c);
    }

    #[test]
    fn rejects_non_array_non_envelope() {
        let body = r#"{ "error": "no jobs found" }"#;
        let err = parse_listings(body, "x").unwrap_err();
        assert!(err.contains("expected JSON array"));
    }

    #[test]
    fn invalid_json_returns_err_not_panic() {
        let body = "not json";
        let err = parse_listings(body, "x").unwrap_err();
        assert!(err.contains("invalid JSON"));
    }

    #[test]
    fn expand_env_substitutes_known_var() {
        // SAFETY: setting an env var in a test isn't safe under
        // parallelism, but this test only reads its own write and
        // doesn't observe other tests' state.
        std::env::set_var("CAREERAI_TEST_FAKE_KEY", "secret-123");
        let out = expand_env("Bearer ${CAREERAI_TEST_FAKE_KEY}");
        assert_eq!(out, "Bearer secret-123");
        std::env::remove_var("CAREERAI_TEST_FAKE_KEY");
    }

    #[test]
    fn expand_env_missing_var_collapses_to_empty() {
        let out = expand_env("X=${CAREERAI_DEFINITELY_UNSET_VAR_XYZ};");
        assert_eq!(out, "X=;");
    }

    #[test]
    fn expand_env_passthrough_when_no_placeholder() {
        let out = expand_env("plain string");
        assert_eq!(out, "plain string");
    }
}
