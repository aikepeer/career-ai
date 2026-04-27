//! `CareerAiServer` — the `rmcp` `ServerHandler` that exposes the
//! pipeline as MCP tools and resources.
//!
//! Architecture: the server holds a project `root` (where `config/`,
//! `profile/`, `data/`, `artifacts/` live). Each tool call loads
//! `CoreConfig` fresh, dispatches into `careerai_pipeline`, and converts
//! the typed outcome into a JSON result. Business logic lives in the
//! pipeline crate; this crate is only an adapter.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    AnnotateAble, CallToolResult, Content, Implementation, ListResourcesResult,
    PaginatedRequestParams, ProtocolVersion, RawResource, ReadResourceRequestParams,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{json, Value};

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

use crate::digest::{format_digest_markdown, parse_since};
use crate::error::McpServerError;
use crate::schema::{
    ApplyArgs, ApplyResult, DigestArgs, DigestResult, DiscoverArgs, DiscoverResult, InspectArgs,
    InspectArtifact, InspectEvent, InspectResult, ProfileStatusArgs, ProfileStatusResult,
    RenderArgs, RenderResult, ShortlistArgs, ShortlistEntry, ShortlistResult, TailorArgs,
    TailorResult,
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
        let result = self.do_profile_status()?;
        json_content(&result)
    }

    fn do_profile_status(&self) -> Result<ProfileStatusResult, McpServerError> {
        let path = self.root().join("profile").join("profile.yaml");
        let path_str = path.display().to_string();

        let metadata = std::fs::metadata(&path);
        let exists = metadata.is_ok();
        let last_modified = metadata
            .as_ref()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            });

        if !exists {
            return Ok(ProfileStatusResult {
                path: path_str,
                exists: false,
                valid: false,
                last_modified: None,
                issues: vec!["profile.yaml does not exist; run `careerai profile import`".into()],
            });
        }

        let text = std::fs::read_to_string(&path).map_err(|e| McpServerError::ProfileMissing {
            path: path_str.clone(),
            source: e,
        })?;

        match careerai_profile::Profile::from_yaml(&text) {
            Ok(profile) => {
                let issues = match profile.check() {
                    Ok(()) => Vec::new(),
                    Err(e) => vec![format!("{e}")],
                };
                Ok(ProfileStatusResult {
                    path: path_str,
                    exists: true,
                    valid: issues.is_empty(),
                    last_modified,
                    issues,
                })
            }
            Err(e) => Ok(ProfileStatusResult {
                path: path_str,
                exists: true,
                valid: false,
                last_modified,
                issues: vec![format!("parse error: {e}")],
            }),
        }
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
        let report = pipeline::discover_all(self.root(), &cfg, &args.sources)
            .await
            .map_err(McpServerError::from)?;

        let result = DiscoverResult {
            fetched: report.fetched,
            new_rows: report.new_rows,
            duplicates: report.duplicates,
            errors: report.errors,
        };
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
        let rows = pipeline::shortlist_show(self.root(), i64::from(limit))
            .await
            .map_err(McpServerError::from)?;

        let entries: Vec<ShortlistEntry> = rows
            .into_iter()
            .filter_map(|l| {
                // f64 -> f32 lossy conversion is fine here: scores are
                // [0.0, 1.0] cosine similarities; f32's ~7-digit
                // precision is more than sufficient for display.
                #[allow(clippy::cast_possible_truncation)]
                let score = l.score.map(|s| s as f32);
                if let (Some(min), Some(s)) = (args.min_score, score) {
                    if s < min {
                        return None;
                    }
                }
                Some(ShortlistEntry {
                    listing_id: l.id,
                    title: l.title,
                    company: l.company,
                    url: l.url,
                    source: l.source,
                    score,
                })
            })
            .collect();

        json_content(&ShortlistResult { entries })
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
        let outcome = pipeline::tailor_one(self.root(), &cfg, &args.listing_id)
            .await
            .map_err(McpServerError::from)?;

        let result = TailorResult {
            application_id: outcome.application_id.clone(),
            diff_summary: format!("tailored {} @ {}", outcome.listing_title, outcome.company),
        };
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
        let outcome = pipeline::render_one(self.root(), &cfg, &args.application_id)
            .await
            .map_err(McpServerError::from)?;

        let result = RenderResult {
            application_id: outcome.application_id,
            docx_path: outcome.resume_docx.display().to_string(),
            pdf_path: outcome.resume_pdf.display().to_string(),
            cover_docx_path: outcome.cover_docx.display().to_string(),
        };
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
        // Constant-time-ish equality is unnecessary here (the token is not
        // a secret — it's a tripwire); a plain string compare is fine.
        if !args.dry_run {
            let token = args.confirm.as_deref().unwrap_or_default();
            if token != "I_UNDERSTAND_TOS_RISK" {
                return Err(McpServerError::AutoSubmitNotConfirmed.into());
            }
        }

        let cfg = self.load_cfg()?;
        let auto_submit_override = if args.dry_run {
            Some(false)
        } else {
            Some(true)
        };

        let outcome = pipeline::apply_one(
            self.root(),
            &cfg,
            &args.application_id,
            auto_submit_override,
        )
        .await
        .map_err(McpServerError::from)?;

        let (kind, would_submit, note) = match &outcome.outcome {
            careerai_submit::SubmitOutcome::Submitted { remote_id } => {
                ("Submitted", None, Some(format!("remote_id={remote_id}")))
            }
            careerai_submit::SubmitOutcome::DryRun { payload_summary } => {
                ("DryRun", Some(payload_summary.clone()), None)
            }
            careerai_submit::SubmitOutcome::Drafted { note } => {
                ("Drafted", None, Some(note.clone()))
            }
            careerai_submit::SubmitOutcome::Skipped { reason } => {
                ("Skipped", None, Some(reason.clone()))
            }
        };

        json_content(&ApplyResult {
            application_id: outcome.application_id,
            source: outcome.source,
            outcome: kind.to_string(),
            would_submit,
            note,
        })
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
        let report = pipeline::inspect_show(self.root(), &args.application_id)
            .await
            .map_err(McpServerError::from)?;

        let events = report
            .events
            .into_iter()
            .map(|e| InspectEvent {
                from_state: e.from_state,
                to_state: e.to_state,
                note: e.note,
                created_at: e
                    .created_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            })
            .collect();

        let artifacts = report
            .artifacts
            .into_iter()
            .map(|a| InspectArtifact {
                kind: a.kind,
                path: a.path,
                bytes: a.bytes,
            })
            .collect();

        json_content(&InspectResult {
            application_id: report.application.id,
            listing_title: report.listing_title,
            listing_company: report.listing_company,
            listing_source: report.listing_source,
            state: report.application.state,
            events,
            artifacts,
        })
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
        let dur = parse_since(&args.since)
            .map_err(|e| McpServerError::Pipeline(format!("parse since: {e}")))?;
        let report = pipeline::digest_summary(self.root(), dur)
            .await
            .map_err(McpServerError::from)?;

        let markdown = format_digest_markdown(&args.since, &report);

        let mut per_source = std::collections::BTreeMap::new();
        for (k, v) in &report.per_source {
            per_source.insert(k.clone(), v.total);
        }

        json_content(&DigestResult {
            markdown,
            since_iso: report.since_iso,
            discovered: report.discovered,
            matched: report.matched,
            shortlisted: report.shortlisted,
            drafted: report.drafted,
            submitted: report.submitted,
            failed: report.failed,
            responded: report.responded,
            per_source,
            last_tick: report.last_tick,
        })
    }

    // ---------- resource handlers ----------

    #[allow(clippy::unused_self)]
    fn list_resources_static(&self) -> Vec<Resource> {
        vec![
            RawResource::new("careerai://profile", "profile.yaml").no_annotation(),
            RawResource::new("careerai://shortlist/today", "today's shortlist (JSON)")
                .no_annotation(),
        ]
    }

    fn read_profile_resource(&self, uri: &str) -> Result<ReadResourceResult, McpServerError> {
        let path = self.root().join("profile").join("profile.yaml");
        let text = std::fs::read_to_string(&path).map_err(|e| McpServerError::ProfileMissing {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text,
            uri.to_string(),
        )]))
    }

    async fn read_shortlist_resource(
        &self,
        uri: &str,
        _date: &str,
    ) -> Result<ReadResourceResult, McpServerError> {
        // We don't index shortlist by date yet; the pipeline only stores
        // the current shortlist. Return whatever's currently in state
        // `shortlisted`.
        let rows = pipeline::shortlist_show(self.root(), 500)
            .await
            .map_err(McpServerError::from)?;
        let body = serde_json::to_string_pretty(&rows)
            .map_err(|e| McpServerError::Pipeline(format!("serialize shortlist: {e}")))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            body,
            uri.to_string(),
        )]))
    }

    async fn read_artifacts_resource(
        &self,
        uri: &str,
        application_id: &str,
    ) -> Result<ReadResourceResult, McpServerError> {
        let report = pipeline::inspect_show(self.root(), application_id)
            .await
            .map_err(McpServerError::from)?;

        let value = json!({
            "application_id": report.application.id,
            "state": report.application.state,
            "artifacts": report.artifacts.iter().map(|a| json!({
                "kind": a.kind,
                "path": a.path,
                "bytes": a.bytes,
            })).collect::<Vec<_>>(),
        });

        let body = serde_json::to_string_pretty(&value)
            .map_err(|e| McpServerError::Pipeline(format!("serialize artifacts: {e}")))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            body,
            uri.to_string(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for CareerAiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "career-ai pipeline as MCP. Discover jobs, score them against the \
             profile, tailor + render resume/cover letter, and submit through \
             a dry-run gate. Tools: careerai_profile_status, careerai_discover, \
             careerai_shortlist, careerai_tailor, careerai_render, \
             careerai_apply (defaults to dry_run; real submit needs an \
             explicit confirm token), careerai_inspect, careerai_digest. \
             Resources: careerai://profile, careerai://shortlist/{date}, \
             careerai://artifacts/{application_id}."
                .to_string(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: self.list_resources_static(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri.as_str();

        // careerai://profile
        if uri == "careerai://profile" {
            return self.read_profile_resource(uri).map_err(Into::into);
        }

        // careerai://shortlist/{date}
        if let Some(rest) = uri.strip_prefix("careerai://shortlist/") {
            let date = rest.split('?').next().unwrap_or(rest);
            return self
                .read_shortlist_resource(uri, date)
                .await
                .map_err(Into::into);
        }

        // careerai://artifacts/{application_id}
        if let Some(rest) = uri.strip_prefix("careerai://artifacts/") {
            let app_id = rest.split('?').next().unwrap_or(rest);
            if app_id.is_empty() {
                return Err(McpServerError::ResourceNotFound(uri.to_string()).into());
            }
            return self
                .read_artifacts_resource(uri, app_id)
                .await
                .map_err(Into::into);
        }

        Err(McpServerError::ResourceNotFound(uri.to_string()).into())
    }
}

/// Helper: serialize a value as JSON and wrap it in a single text content
/// block. The MCP transport currently doesn't have a structured-result
/// shape that's universally supported, so plain JSON-in-text is the
/// safest interop.
fn json_content<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let body = serde_json::to_string(value)
        .map_err(|e| McpServerError::Pipeline(format!("serialize result: {e}")))?;
    Ok(CallToolResult::success(vec![Content::text(body)]))
}

// Keep the unused `Value` import suppressed when we expand handlers later.
#[allow(dead_code)]
fn _force_value_used(_: Value) {}
