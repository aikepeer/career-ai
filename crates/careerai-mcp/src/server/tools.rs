//! Tool handler implementations — the `do_*` methods that contain
//! actual pipeline dispatch logic. Kept separate from the thin
//! `#[tool]` wrapper stanzas in the root `server.rs` so the proc-macro
//! attributed impl block stays short.

use std::collections::BTreeMap;

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

use crate::digest::format_digest_markdown;
use crate::error::McpServerError;
use crate::schema::{
    ApplyResult, DigestResult, DiscoverResult, InspectArtifact, InspectEvent, InspectResult,
    ProfileStatusResult, RenderResult, ShortlistEntry, ShortlistResult, TailorResult,
};

use super::CareerAiServer;

impl CareerAiServer {
    pub(crate) async fn do_profile_status(&self) -> Result<ProfileStatusResult, McpServerError> {
        let path = self.root().join("profile").join("profile.yaml");
        let path_str = path.display().to_string();

        // Distinguish "file truly absent" (a normal, expected state on a
        // fresh checkout) from real I/O failures (permission denied,
        // broken symlink, transient FS error). The former is reported
        // as a structured `exists: false` result; the latter is surfaced
        // as an MCP error so the operator sees the real cause instead
        // of a misleading "does not exist" message.
        //
        // Uses `tokio::fs` so the async reactor is never blocked even on
        // a slow disk; profile.yaml is small in practice but the rule is
        // "no sync I/O on the reactor".
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => Some(m),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(McpServerError::ProfileIo {
                    path: path_str,
                    source: e,
                });
            }
        };

        let last_modified = metadata.as_ref().and_then(|m| m.modified().ok()).map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });

        if metadata.is_none() {
            return Ok(ProfileStatusResult {
                path: path_str,
                exists: false,
                valid: false,
                last_modified: None,
                issues: vec!["profile.yaml does not exist; run `careerai profile import`".into()],
            });
        }

        let text =
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| McpServerError::ProfileMissing {
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

    pub(crate) async fn do_discover(
        &self,
        cfg: &CoreConfig,
        sources: &[String],
    ) -> Result<DiscoverResult, McpServerError> {
        let report = pipeline::discover_all(self.root(), cfg, sources)
            .await
            .map_err(McpServerError::from)?;
        Ok(DiscoverResult {
            fetched: report.fetched,
            new_rows: report.new_rows,
            duplicates: report.duplicates,
            errors: report.errors,
        })
    }

    pub(crate) async fn do_shortlist(
        &self,
        limit: i64,
        min_score: Option<f32>,
    ) -> Result<ShortlistResult, McpServerError> {
        let rows = pipeline::shortlist_show(self.root(), limit)
            .await
            .map_err(McpServerError::from)?;

        let entries: Vec<ShortlistEntry> = rows
            .into_iter()
            .filter_map(|l| {
                #[allow(clippy::cast_possible_truncation)]
                let score = l.score.map(|s| s as f32);
                if let (Some(min), Some(s)) = (min_score, score) {
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

        Ok(ShortlistResult { entries })
    }

    pub(crate) async fn do_tailor(
        &self,
        cfg: &CoreConfig,
        listing_id: &str,
    ) -> Result<TailorResult, McpServerError> {
        let outcome = pipeline::tailor_one(self.root(), cfg, listing_id)
            .await
            .map_err(McpServerError::from)?;
        Ok(TailorResult {
            application_id: outcome.application_id.clone(),
            diff_summary: format!("tailored {} @ {}", outcome.listing_title, outcome.company),
        })
    }

    pub(crate) async fn do_render(
        &self,
        cfg: &CoreConfig,
        application_id: &str,
    ) -> Result<RenderResult, McpServerError> {
        let outcome = pipeline::render_one(self.root(), cfg, application_id)
            .await
            .map_err(McpServerError::from)?;
        Ok(RenderResult {
            application_id: outcome.application_id,
            docx_path: outcome.resume_docx.display().to_string(),
            pdf_path: outcome.resume_pdf.display().to_string(),
            cover_docx_path: outcome.cover_docx.display().to_string(),
        })
    }

    pub(crate) async fn do_apply(
        &self,
        cfg: &CoreConfig,
        application_id: &str,
        dry_run: bool,
    ) -> Result<ApplyResult, McpServerError> {
        let auto_submit_override = if dry_run { Some(false) } else { Some(true) };

        let outcome = pipeline::apply_one(self.root(), cfg, application_id, auto_submit_override)
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

        Ok(ApplyResult {
            application_id: outcome.application_id,
            source: outcome.source,
            outcome: kind.to_string(),
            would_submit,
            note,
        })
    }

    pub(crate) async fn do_inspect(
        &self,
        application_id: &str,
    ) -> Result<InspectResult, McpServerError> {
        let report = pipeline::inspect_show(self.root(), application_id)
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

        Ok(InspectResult {
            application_id: report.application.id,
            listing_title: report.listing_title,
            listing_company: report.listing_company,
            listing_source: report.listing_source,
            state: report.application.state,
            events,
            artifacts,
        })
    }

    pub(crate) async fn do_digest(&self, since: &str) -> Result<DigestResult, McpServerError> {
        let dur = crate::digest::parse_since(since)
            .map_err(|e| McpServerError::Pipeline(format!("parse since: {e}")))?;
        let report = pipeline::digest_summary(self.root(), dur)
            .await
            .map_err(McpServerError::from)?;

        let markdown = format_digest_markdown(since, &report);

        let mut per_source = BTreeMap::new();
        for (k, v) in &report.per_source {
            per_source.insert(k.clone(), v.total);
        }

        Ok(DigestResult {
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
}
