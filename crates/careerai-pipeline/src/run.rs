//! `run_pipeline` — the full shortlist → tailor → render → apply
//! orchestration used by `careerai run` and the dashboard "Run All"
//! control. Returns a serializable report so callers can render a
//! one-shot summary without re-querying the DB.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use careerai_core::config::CoreConfig;
use careerai_submit::SubmitOutcome;

use crate::apply::{apply_all_detailed, AppliedOutcome, ApplyFailure};
use crate::{match_all, render_one, shortlist_show, tailor_one};

/// Aggregated outcome of one full pipeline run.
#[derive(Debug, Default, Serialize)]
pub struct RunReport {
    /// Listings found in `shortlisted` state and eligible for tailoring.
    pub shortlisted: usize,
    /// Listings successfully tailored this run.
    pub tailored: usize,
    /// Applications successfully rendered this run.
    pub rendered: usize,
    /// Submit-stage summary (submitted/drafted/dry-run/skipped/failed).
    pub apply: ApplyReport,
    /// Every stage failure encountered, in order.
    pub failures: Vec<RunFailure>,
}

/// Submit-stage summary with per-source breakdown.
#[derive(Debug, Default, Serialize)]
pub struct ApplyReport {
    pub submitted: usize,
    pub drafted: usize,
    pub dry_run: usize,
    pub skipped: usize,
    pub failed: usize,
    pub per_source: BTreeMap<String, ApplySourceReport>,
}

#[derive(Debug, Default, Serialize)]
pub struct ApplySourceReport {
    pub submitted: usize,
    pub drafted: usize,
    pub dry_run: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct RunFailure {
    pub stage: String,
    pub id: String,
    pub source: Option<String>,
    pub error: String,
}

/// Drive the full pipeline from match through apply.
///
/// `auto_submit` is the operator's override for the submit stage:
/// `false` forces dry-run (no network writes), `true` forces live where
/// the per-source gate and LinkedIn `interactive_only` still permit.
/// Individual stage failures are recorded in `report.failures` and never
/// abort the rest of the run.
pub async fn run_pipeline(root: &Path, cfg: &CoreConfig, auto_submit: bool) -> Result<RunReport> {
    let match_report = match_all(root, cfg, false).await?;
    tracing::info!(
        shortlisted = match_report.shortlisted,
        filtered_out = match_report.filtered_out,
        "match stage complete",
    );

    let shortlisted = shortlist_show(root, 10_000).await?;
    let mut report = RunReport {
        shortlisted: shortlisted.len(),
        ..RunReport::default()
    };

    let mut tailored_ids = Vec::with_capacity(shortlisted.len());
    for listing in &shortlisted {
        match tailor_one(root, cfg, &listing.id).await {
            Ok(outcome) => {
                report.tailored += 1;
                tailored_ids.push(outcome.application_id);
            }
            Err(e) => report.failures.push(RunFailure {
                stage: "tailor".to_string(),
                id: listing.id.clone(),
                source: Some(listing.source.clone()),
                error: format!("{e:#}"),
            }),
        }
    }

    let mut rendered_ids = Vec::with_capacity(tailored_ids.len());
    for application_id in &tailored_ids {
        match render_one(root, cfg, application_id).await {
            Ok(_) => {
                report.rendered += 1;
                rendered_ids.push(application_id.clone());
            }
            Err(e) => report.failures.push(RunFailure {
                stage: "render".to_string(),
                id: application_id.clone(),
                source: None,
                error: format!("{e:#}"),
            }),
        }
    }

    let (outcomes, failures) = apply_all_detailed(root, cfg, None, Some(auto_submit)).await?;
    let (apply, apply_failures) = aggregate_apply(outcomes, failures);
    report.apply = apply;
    report.failures.extend(apply_failures);

    Ok(report)
}

/// Pure aggregation of submit-stage outcomes + failures into the report
/// shape. Kept free of I/O so the per-source accounting is unit-testable.
fn aggregate_apply(
    outcomes: Vec<AppliedOutcome>,
    failures: Vec<ApplyFailure>,
) -> (ApplyReport, Vec<RunFailure>) {
    let mut report = ApplyReport::default();
    for outcome in outcomes {
        let per_source = report.per_source.entry(outcome.source.clone()).or_default();
        match outcome.outcome {
            SubmitOutcome::Submitted { .. } => {
                report.submitted += 1;
                per_source.submitted += 1;
            }
            SubmitOutcome::Drafted { .. } => {
                report.drafted += 1;
                per_source.drafted += 1;
            }
            SubmitOutcome::DryRun { .. } => {
                report.dry_run += 1;
                per_source.dry_run += 1;
            }
            SubmitOutcome::Skipped { .. } => {
                report.skipped += 1;
                per_source.skipped += 1;
            }
        }
    }

    let mut run_failures = Vec::with_capacity(failures.len());
    for failure in failures {
        report.failed += 1;
        report
            .per_source
            .entry(failure.source.clone())
            .or_default()
            .failed += 1;
        run_failures.push(RunFailure {
            stage: "apply".to_string(),
            id: failure.application_id,
            source: Some(failure.source),
            error: failure.error,
        });
    }

    (report, run_failures)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn outcome(source: &str, id: &str, outcome: SubmitOutcome) -> AppliedOutcome {
        AppliedOutcome {
            application_id: id.to_string(),
            source: source.to_string(),
            outcome,
        }
    }

    #[test]
    fn aggregate_apply_counts_submitted_drafted_dry_run_skipped_and_failed() {
        let outcomes = vec![
            outcome(
                "greenhouse",
                "a",
                SubmitOutcome::Submitted {
                    remote_id: "r1".into(),
                },
            ),
            outcome(
                "greenhouse",
                "b",
                SubmitOutcome::DryRun {
                    payload_summary: "POST /apply".into(),
                },
            ),
            outcome(
                "linkedin",
                "c",
                SubmitOutcome::Drafted {
                    note: "review".into(),
                },
            ),
            outcome(
                "lever",
                "d",
                SubmitOutcome::Skipped {
                    reason: "disabled".into(),
                },
            ),
        ];
        let failures = vec![ApplyFailure {
            application_id: "e".into(),
            source: "ashby".into(),
            error: "boom".into(),
        }];

        let (report, run_failures) = aggregate_apply(outcomes, failures);

        assert_eq!(report.submitted, 1);
        assert_eq!(report.dry_run, 1);
        assert_eq!(report.drafted, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(report.per_source["greenhouse"].submitted, 1);
        assert_eq!(report.per_source["greenhouse"].dry_run, 1);
        assert_eq!(report.per_source["linkedin"].drafted, 1);
        assert_eq!(report.per_source["lever"].skipped, 1);
        assert_eq!(report.per_source["ashby"].failed, 1);
        assert_eq!(run_failures.len(), 1);
        assert_eq!(run_failures[0].stage, "apply");
        assert_eq!(run_failures[0].id, "e");
    }
}
