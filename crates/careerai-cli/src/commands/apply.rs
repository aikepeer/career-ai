//! `careerai apply` and `careerai applied` — submit prepared
//! applications (dry-run by default) and inspect submission history.
//!
//! Also owns `map_apply_error_to_exit_code`, which is shared by the
//! inspect-path so its typed-error → exit-code mapping stays in one
//! place.

use std::path::Path;

use anyhow::Result;

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

pub async fn run(
    cwd: &Path,
    cfg: &CoreConfig,
    application_id: Option<String>,
    all: bool,
    auto_submit: bool,
    source_filter: Option<&str>,
) -> Result<()> {
    let auto_submit_override = if auto_submit { Some(true) } else { Some(false) };

    match (application_id, all) {
        (None, false) => {
            anyhow::bail!("specify --all or an application id");
        }
        (Some(_), true) => {
            anyhow::bail!("pass either --all or an application id, not both");
        }
        (Some(id), false) => match pipeline::apply_one(cwd, cfg, &id, auto_submit_override).await {
            Ok(outcome) => {
                print_apply_line(&outcome, auto_submit);
            }
            Err(e) => {
                tracing::error!(error = %format_args!("{e:#}"), "apply failed");
                std::process::exit(map_apply_error_to_exit_code(&e));
            }
        },
        (None, true) => {
            match pipeline::apply_all(cwd, cfg, source_filter, auto_submit_override).await {
                Ok(outcomes) => {
                    if outcomes.is_empty() {
                        println!(
                            "(no rendered/prepared applications — run `careerai render` first)"
                        );
                        return Ok(());
                    }
                    for outcome in &outcomes {
                        print_apply_line(outcome, auto_submit);
                    }
                }
                Err(e) => {
                    tracing::error!(error = %format_args!("{e:#}"), "apply --all failed");
                    std::process::exit(map_apply_error_to_exit_code(&e));
                }
            }
        }
    }
    Ok(())
}

fn print_apply_line(outcome: &pipeline::AppliedOutcome, auto_submit: bool) {
    let tag = if auto_submit {
        "[live]   "
    } else {
        "[dry-run]"
    };
    match &outcome.outcome {
        careerai_submit::SubmitOutcome::DryRun { .. } => {
            println!(
                "{tag} application={} source={} -> DryRun (would_submit logged)",
                outcome.application_id, outcome.source,
            );
        }
        careerai_submit::SubmitOutcome::Submitted { remote_id } => {
            println!(
                "[live]    application={} source={} -> Submitted (remote={})",
                outcome.application_id, outcome.source, remote_id,
            );
        }
        careerai_submit::SubmitOutcome::Skipped { reason } => {
            println!(
                "[skip]    application={} source={} -> Skipped ({})",
                outcome.application_id, outcome.source, reason,
            );
        }
        careerai_submit::SubmitOutcome::Drafted { note } => {
            println!(
                "[draft]   application={} source={} -> Drafted ({}; run `careerai review` to confirm)",
                outcome.application_id, outcome.source, note,
            );
        }
    }
}

pub async fn run_applied(cwd: &Path, source_filter: Option<&str>, limit: i64) -> Result<()> {
    let rows = pipeline::applied_show(cwd, source_filter, limit).await?;
    if rows.is_empty() {
        println!("(no submitted applications yet)");
        return Ok(());
    }
    // Columnar header.
    println!(
        "{:<38}  {:<12}  {:<10}  UPDATED_AT",
        "APPLICATION_ID", "SOURCE", "STATE"
    );
    // We need the listing's source per row; fetch once per row.
    let pool = pipeline::open_pool(cwd).await?;
    for app in rows {
        let source = careerai_db::queries::find_by_id(&pool, &app.listing_id)
            .await
            .map_or_else(|_| "?".to_owned(), |l| l.source);
        println!(
            "{:<38}  {:<12}  {:<10}  {}",
            app.id,
            source,
            app.state,
            app.updated_at.format("%Y-%m-%dT%H:%M:%SZ"),
        );
    }
    Ok(())
}

/// Map an apply/inspect-path error to a stable exit code.
///
/// - 2: application / listing not found
/// - 3: application in wrong state (not 'rendered'/'prepared')
/// - 5: source disabled (per-source config gate)
/// - 6: ATS upstream HTTP failure
/// - 7: unknown source (no submitter registered)
/// - 1: anything else
pub(crate) fn map_apply_error_to_exit_code(err: &anyhow::Error) -> i32 {
    // Walk the cause chain and match on typed errors only. The
    // earlier stringly-typed fallback (`msg.starts_with("application
    // not found:")`) was fragile: any `.context("…")` wrapping
    // prepended text and silently broke the match. The typed downcast
    // path covers every real path because `apply_one` / `inspect_show`
    // wrap `careerai_db::DbError::NotFound` directly, and
    // `submit_application` returns `careerai_submit::SubmitError`.
    for cause in err.chain() {
        if let Some(se) = cause.downcast_ref::<careerai_submit::SubmitError>() {
            return match se {
                careerai_submit::SubmitError::BadState { .. } => 3,
                careerai_submit::SubmitError::SourceDisabled(_) => 5,
                careerai_submit::SubmitError::UnknownSource(_) => 7,
                careerai_submit::SubmitError::Http(_)
                | careerai_submit::SubmitError::HttpStatus { .. } => 6,
                careerai_submit::SubmitError::Db(careerai_db::DbError::NotFound(_)) => 2,
                _ => 1,
            };
        }
        if let Some(careerai_db::DbError::NotFound(_)) =
            cause.downcast_ref::<careerai_db::DbError>()
        {
            return 2;
        }
    }
    1
}
