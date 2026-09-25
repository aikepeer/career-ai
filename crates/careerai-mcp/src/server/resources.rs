//! Resource listing and reading logic — `resources/list`, `resources/templates/list`,
//! and `resources/read` dispatching. Kept separate so `server.rs` stays under 300 LOC.

use rmcp::model::{
    AnnotateAble, RawResource, RawResourceTemplate, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate,
};
use serde_json::json;

use careerai_pipeline as pipeline;

use crate::error::McpServerError;
use crate::schema::CompactListing;

use super::CareerAiServer;

impl CareerAiServer {
    /// Concrete (non-templated) resources discoverable via
    /// `resources/list`. Parameterized URIs (`careerai://shortlist/{date}`,
    /// `careerai://artifacts/{application_id}`) are advertised separately
    /// via `resources/templates/list` — see `list_resource_templates_static`.
    #[allow(clippy::unused_self)]
    pub(crate) fn list_resources_static(&self) -> Vec<Resource> {
        // Only the truly concrete `careerai://profile` URI lives here.
        // Date-parameterized shortlist URIs and per-application artifacts
        // URIs are advertised via `resources/templates/list` instead;
        // listing both shapes confused MCP clients about which is
        // canonical (Copilot review on PR #18). The friendly alias
        // `careerai://shortlist/today` is still accepted by
        // `read_resource` for backwards compat — it just isn't
        // duplicated in the discovery surface.
        vec![RawResource::new("careerai://profile", "profile.yaml").no_annotation()]
    }

    /// Resource templates (URI patterns) advertised via
    /// `resources/templates/list`. MCP clients use these to discover
    /// parameterized resources that can't be listed exhaustively
    /// (every date / every application id).
    #[allow(clippy::unused_self)]
    pub(crate) fn list_resource_templates_static(&self) -> Vec<ResourceTemplate> {
        vec![
            RawResourceTemplate::new("careerai://shortlist/{date}", "shortlist by date")
                .no_annotation(),
            RawResourceTemplate::new(
                "careerai://artifacts/{application_id}",
                "artifacts for an application",
            )
            .no_annotation(),
        ]
    }

    pub(crate) async fn read_profile_resource(
        &self,
        uri: &str,
    ) -> Result<ReadResourceResult, McpServerError> {
        let path = careerai_core::paths::profile_path(self.root());
        let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpServerError::ResourceMissing {
                uri: uri.to_string(),
                source: e,
            }
        })?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text,
            uri.to_string(),
        )]))
    }

    pub(crate) async fn read_shortlist_resource(
        &self,
        uri: &str,
        date_seg: &str,
    ) -> Result<ReadResourceResult, McpServerError> {
        // Validate the {date} segment so a malformed URI surfaces as an
        // `invalid_params` error instead of being silently swallowed.
        // The pipeline doesn't yet support per-day filtering, so a valid
        // date is accepted but logged as ignored — the caller still gets
        // today's shortlist. The literal `today` is a friendly alias.
        if date_seg != "today" {
            match chrono::NaiveDate::parse_from_str(date_seg, "%Y-%m-%d") {
                Ok(_) => {
                    tracing::warn!(
                        date = %date_seg,
                        "shortlist date segment parsed but ignored: pipeline does not yet \
                         support per-day filtering — returning current shortlist"
                    );
                }
                Err(e) => {
                    return Err(McpServerError::InvalidArgument(format!(
                        "shortlist date segment must be `today` or YYYY-MM-DD (got `{date_seg}`): {e}"
                    )));
                }
            }
        }

        let rows = pipeline::shortlist_show(self.root(), 500)
            .await
            .map_err(McpServerError::from)?;

        // Project to the compact shape: drop `description` (free-form
        // HTML/text JD) and `raw_json` (full ATS payload) which are
        // bulky and not useful for LLM triage. Same fields the
        // `careerai_shortlist` tool returns, plus `location`.
        let compact: Vec<CompactListing> = rows
            .into_iter()
            .map(|l| {
                #[allow(clippy::cast_possible_truncation)]
                let score = l.score.map(|s| s as f32);
                CompactListing {
                    listing_id: l.id,
                    title: l.title,
                    company: l.company,
                    url: l.url,
                    source: l.source,
                    score,
                    location: l.location,
                }
            })
            .collect();

        let body = serde_json::to_string_pretty(&compact)
            .map_err(|e| McpServerError::Pipeline(format!("serialize shortlist: {e}")))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            body,
            uri.to_string(),
        )]))
    }

    pub(crate) async fn read_artifacts_resource(
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
