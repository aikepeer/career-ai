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
use tracing::{debug, info};

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

/// Per-name interner for `(name_static, rate_limiter)` pairs.
///
/// `build_sources()` is invoked on every cron tick, which means
/// `McpJobsSource::new` is called repeatedly with the same `cfg.name`
/// over the daemon's lifetime. Without this cache:
///   1. Every call would `Box::leak` a fresh copy of the source name —
///      one slow leak per tick, per source. Bounded but unbounded over
///      uptime; pre-fix doc comment claimed otherwise and was wrong.
///   2. Every call would build a new `RateLimiter` with a full bucket,
///      so the per-minute cap never fired in practice.
///
/// With the cache: one allocation per distinct source name, and the
/// token-bucket state is shared across reconstructions so the cap is
/// honored.
type InternedState = (&'static str, Option<Arc<ReadRateLimiter>>);
type InternMap = std::sync::Mutex<std::collections::HashMap<String, InternedState>>;
static SOURCE_STATE: std::sync::OnceLock<InternMap> = std::sync::OnceLock::new();

fn intern_source_state(name: &str, rate_per_minute: u32) -> InternedState {
    let map = SOURCE_STATE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // `lock()` only fails if a previous holder panicked. The state we
    // keep is just `(static str, Arc<RateLimiter>)`; recovery is safe.
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = guard.get(name) {
        return entry.clone();
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let rl = NonZeroU32::new(rate_per_minute)
        .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
    guard.insert(name.to_owned(), (leaked, rl.clone()));
    (leaked, rl)
}

#[derive(Debug)]
pub struct McpJobsSource {
    cfg: McpSourceConfig,
    /// `&'static str` produced via a name-keyed interner backed by
    /// `Box::leak`. Constructing two `McpJobsSource` instances with the
    /// same `cfg.name` returns the same `&'static str` (one allocation
    /// per distinct source name, for the lifetime of the process). The
    /// scheduler invokes `build_sources()` on every cron tick — without
    /// the cache that would leak a fresh string every tick.
    name_static: &'static str,
    /// Shared with all other `McpJobsSource` instances that have the
    /// same `cfg.name`. The token-bucket state lives in the `Arc`, so a
    /// new construction (e.g. on the next cron tick via
    /// `build_sources()`) does not reset the bucket — the per-minute
    /// cap is honored across ticks.
    rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl McpJobsSource {
    #[must_use]
    pub fn new(cfg: McpSourceConfig) -> Self {
        let (name_static, rate_limiter) = intern_source_state(&cfg.name, cfg.rate_per_minute);
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

async fn call_and_parse(
    src: &McpJobsSource,
    svc: &RunningService<RoleClient, ProbeClient>,
) -> Result<Vec<RawListing>, SourceError> {
    let tool_name = src.pick_tool(svc).await?;
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

/// Extract the first text-content block from a `tools/call` result, if
/// any. Most servers emit `{ "content": [{ "type": "text", "text": "..." }] }`.
fn first_text_content(content: &[rmcp::model::Content]) -> Option<String> {
    content.iter().find_map(|c| match &c.raw {
        rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
        _ => None,
    })
}

fn build_command(cfg: &McpTransportConfig) -> Command {
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
    // Strip HTML to plain text — community MCPs (e.g. mcp-linkedin)
    // return marketing HTML in `description`, and the matcher
    // embeddings work much better on clean text. Mirrors the same
    // pre-processing done by every other adapter (greenhouse, lever,
    // remoteok, remotive, naukri).
    let description = pick_string(item, &["description", "summary", "snippet"])
        .map(|s| crate::util::html_to_text(&s))
        .unwrap_or_default();
    let external_id = pick_string(item, &["id", "job_id", "external_id"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| derive_external_id(source_name, &url, &title, &company));
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

// `parse_posted_at` was previously used to populate a discarded
// `_posted_at_unused` local. `RawListing` has no `posted_at` field; the
// value was thrown away. Removed in the PR-19 review pass — if the
// schema later grows a `posted_at`, reintroduce the parser then.

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
    fn external_id_differs_across_sources_for_same_url() {
        // Two MCP sources can legitimately surface the same job (same
        // URL, title, company, no upstream id). The downstream upsert
        // key is `(source, external_id)`; if the SHA didn't include the
        // source name, both rows would collapse into one and we'd
        // silently drop the second source's listing.
        let a = derive_external_id("source-one", "u", "t", "c");
        let b = derive_external_id("source-two", "u", "t", "c");
        assert_ne!(a, b, "external_id must vary by source_name");
    }

    #[test]
    fn description_html_is_stripped_to_plain_text() {
        // mcp-linkedin-style payload: HTML inside `description`. The
        // matcher embeddings expect plain text; assert we strip tags
        // the same way greenhouse / lever / remoteok do.
        let body = r#"[{
            "title": "X",
            "company": "Y",
            "url": "https://example.com/x",
            "description": "<p>Build <strong>LLM</strong> apps.</p>"
        }]"#;
        let listings = parse_listings(body, "src").expect("parse");
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].description, "Build LLM apps.");
    }

    #[test]
    fn intern_source_state_reuses_static_name_and_limiter() {
        // Two `McpJobsSource` constructed with the same `name` must
        // share the same `&'static str` (so the cache is hit, no
        // per-tick leak) and the same rate-limiter `Arc` (so the
        // token-bucket state survives `build_sources()` rebuild).
        let cfg = |rpm: u32| McpSourceConfig {
            name: "intern-test-shared".into(),
            enabled: true,
            submit_enabled: false,
            cron: None,
            rate_per_minute: rpm,
            mcp: McpTransportConfig::default(),
        };
        let a = McpJobsSource::new(cfg(60));
        let b = McpJobsSource::new(cfg(60));
        // Same `&'static str` (pointer identity, not just string equality).
        assert!(
            std::ptr::eq(a.name(), b.name()),
            "name_static must be interned across constructions"
        );
        // Same limiter `Arc`.
        let arc_a = a.rate_limiter.as_ref().expect("limiter set");
        let arc_b = b.rate_limiter.as_ref().expect("limiter set");
        assert!(
            Arc::ptr_eq(arc_a, arc_b),
            "rate_limiter Arc must be shared across constructions"
        );
    }

    #[test]
    fn parse_listings_propagates_error_instead_of_empty() {
        // Regression: previously `run_call` swallowed parse errors and
        // returned an empty Vec, hiding upstream schema breakage from
        // the operator. The mapper itself returns Result; assert that
        // a malformed body surfaces the error rather than `Ok(vec![])`.
        let err = parse_listings("not json", "x").unwrap_err();
        assert!(err.contains("invalid JSON"), "got: {err}");
        let err = parse_listings(r#"{"unexpected": true}"#, "x").unwrap_err();
        assert!(err.contains("expected JSON array"), "got: {err}");
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
