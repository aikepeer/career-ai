//! `CareerAiServer` — the `rmcp` `ServerHandler` that exposes the
//! pipeline as MCP tools and resources.
//!
//! Architecture: the server holds a project `root` (where `config/`,
//! `profile/`, `data/`, `artifacts/` live). Each tool call loads
//! `CoreConfig` fresh, dispatches into `careerai_pipeline`, and converts
//! the typed outcome into a JSON result. Business logic lives in the
//! pipeline crate; this crate is only an adapter.
//!
//! ## Module layout (split to stay under the 300-LOC cap)
//!
//! * `tools.rs`      — tool `do_*` dispatch methods
//! * `resources.rs`  — resource listing/reading logic
//! * `handlers.rs`   — `ServerHandler` trait impl (lifecycle + routing)

mod handlers;
mod resources;
pub(crate) mod tools;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{tool, tool_router, ErrorData as McpError};
use serde_json::Value;

use careerai_core::config::CoreConfig;

use crate::error::McpServerError;
use crate::schema::{
    ApplyArgs, DigestArgs, DiscoverArgs, InspectArgs, ProfileStatusArgs, RenderArgs, ShortlistArgs,
    TailorArgs,
};

/// Career-ai MCP server. Holds the project root and a re-usable rmcp
/// router built from the `#[tool_router]` macro.
///
/// `tool_router` looks unused to rustc (the macros read it via
/// `Self::tool_router()` from a separate impl), so we silence dead-code
/// warnings on it explicitly.
#[derive(Clone)]
pub struct CareerAiServer {
    root: Arc<PathBuf>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for CareerAiServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `ToolRouter` doesn't implement Debug; `finish_non_exhaustive`
        // signals to readers (and clippy) that we know fields are
        // omitted on purpose.
        f.debug_struct("CareerAiServer")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

#[tool_router]
impl CareerAiServer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
            tool_router: Self::tool_router(),
        }
    }

    fn root(&self) -> &Path {
        self.root.as_path()
    }

    fn load_cfg(&self) -> Result<CoreConfig, McpServerError> {
        CoreConfig::load(self.root())
            .map_err(|e| McpServerError::Pipeline(format!("load config: {e}")))
    }

    // ---------- careerai_profile_status ----------

    #[tool(
        description = "Inspect the user's career-ai master profile. Returns the on-disk path \
                       of profile/profile.yaml, whether it exists, whether it parses + \
                       validates, last-modified time, and a list of any validation issues. \
                       Use this before tailoring to confirm the profile is healthy."
    )]
    async fn careerai_profile_status(
        &self,
        Parameters(_args): Parameters<ProfileStatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = self.do_profile_status().await?;
        json_content(&result)
    }

    // ---------- careerai_discover ----------

    #[tool(
        description = "Run the discovery pipeline against one or more sources \
                       (greenhouse / lever / linkedin / indeed / ashby / rss). \
                       Inserts new listings into the SQLite DB; never submits \
                       anything. Pass `sources: []` (or omit) to run every \
                       enabled source. Returns counts of fetched, new, duplicate, \
                       and errored rows."
    )]
    async fn careerai_discover(
        &self,
        Parameters(args): Parameters<DiscoverArgs>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = self.load_cfg()?;
        let result = self.do_discover(&cfg, &args.sources).await?;
        json_content(&result)
    }

    // ---------- careerai_shortlist ----------

    #[tool(
        description = "List shortlisted listings (those that passed hard filters \
                       and scored above the configured threshold). Read-only. \
                       Args: `limit` (default 50, max 500), `min_score` (optional \
                       client-side filter on the cosine score)."
    )]
    async fn careerai_shortlist(
        &self,
        Parameters(args): Parameters<ShortlistArgs>,
    ) -> Result<CallToolResult, McpError> {
        let limit = args.limit.unwrap_or(50).min(500);
        let result = self.do_shortlist(i64::from(limit), args.min_score).await?;
        json_content(&result)
    }

    // ---------- careerai_tailor ----------

    #[tool(description = "Tailor the user's resume + cover letter to a specific \
                       shortlisted listing using the LLM. The diff is constrained: \
                       it can only reorder or rewrite EXISTING bullets from the \
                       master profile (no fabrication of new experience). Persists \
                       a new application row and returns its id.")]
    async fn careerai_tailor(
        &self,
        Parameters(args): Parameters<TailorArgs>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = self.load_cfg()?;
        let result = self.do_tailor(&cfg, &args.listing_id).await?;
        json_content(&result)
    }

    // ---------- careerai_render ----------

    #[tool(
        description = "Render a tailored application to DOCX + PDF on disk via \
                       Tera + pandoc. Requires `pandoc` on PATH. Returns the \
                       absolute paths of the produced artifacts. Fails if the \
                       application is not in state `tailored`."
    )]
    async fn careerai_render(
        &self,
        Parameters(args): Parameters<RenderArgs>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = self.load_cfg()?;
        let result = self.do_render(&cfg, &args.application_id).await?;
        json_content(&result)
    }

    // ---------- careerai_apply ----------

    #[tool(
        description = "Submit a rendered application. SAFETY GATES: defaults to \
                       dry_run=true. Real submission requires BOTH dry_run=false \
                       AND confirm=\"I_UNDERSTAND_TOS_RISK\" (LinkedIn / Indeed \
                       auto-apply violates their ToS). Per-source `submit_enabled` \
                       gates in config still apply even with confirm. \
                       LinkedIn applications are routed to the assist queue \
                       (state `drafted`) instead of being submitted autonomously."
    )]
    async fn careerai_apply(
        &self,
        Parameters(args): Parameters<ApplyArgs>,
    ) -> Result<CallToolResult, McpError> {
        // Safety gate: if dry_run is false, require the literal confirm token.
        if !args.dry_run {
            let token = args.confirm.as_deref().unwrap_or_default();
            if token != "I_UNDERSTAND_TOS_RISK" {
                return Err(McpServerError::AutoSubmitNotConfirmed.into());
            }
        }
        let cfg = self.load_cfg()?;
        let result = self
            .do_apply(&cfg, &args.application_id, args.dry_run)
            .await?;
        json_content(&result)
    }

    // ---------- careerai_inspect ----------

    #[tool(
        description = "Show the full event history + on-disk artifacts for an \
                       application. Read-only. Useful for debugging why a listing \
                       got filtered out, or for listing rendered artifacts before \
                       submission."
    )]
    async fn careerai_inspect(
        &self,
        Parameters(args): Parameters<InspectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = self.do_inspect(&args.application_id).await?;
        json_content(&result)
    }

    // ---------- careerai_digest ----------

    #[tool(
        description = "Render a daily/weekly digest of pipeline activity. Args: \
                       `since` accepts `<n>h`, `<n>d`, `<n>w` (e.g. \"1d\", \"24h\", \
                       \"2w\") or a bare integer interpreted as hours. Returns \
                       both raw counts and a pre-formatted markdown string."
    )]
    async fn careerai_digest(
        &self,
        Parameters(args): Parameters<DigestArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = self.do_digest(&args.since).await?;
        json_content(&result)
    }
}

/// Helper: serialize a value as JSON and wrap it in a single text content
/// block. The MCP transport currently doesn't have a structured-result
/// shape that's universally supported, so plain JSON-in-text is the
/// safest interop.
fn json_content<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let body = serde_json::to_string(value)
        .map_err(|e| McpServerError::Pipeline(format!("serialize result: {e}")))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
}

// Keep the unused `Value` import suppressed when we expand handlers later.
#[allow(dead_code)]
fn _force_value_used(_: Value) {}
