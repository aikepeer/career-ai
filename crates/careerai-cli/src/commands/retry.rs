//! `careerai retry` — reset a `failed` application to its pre-submit
//! state and re-run `apply_one`.

use std::path::Path;

use anyhow::Result;

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;
use careerai_submit::SubmitOutcome;

pub async fn run(cwd: &Path, cfg: &CoreConfig, application_id: &str) -> Result<()> {
    match pipeline::retry_application(cwd, cfg, application_id).await {
        Ok(outcome) => match &outcome.outcome {
            SubmitOutcome::DryRun { .. } => {
                println!(
                    "[dry-run] application={} source={} -> DryRun (retry would_submit logged)",
                    outcome.application_id, outcome.source,
                );
            }
            SubmitOutcome::Submitted { remote_id } => {
                println!(
                    "[live]    application={} source={} -> Submitted (remote={})",
                    outcome.application_id, outcome.source, remote_id,
                );
            }
            SubmitOutcome::Skipped { reason } => {
                println!(
                    "[skip]    application={} source={} -> Skipped ({})",
                    outcome.application_id, outcome.source, reason,
                );
            }
            SubmitOutcome::Drafted { note } => {
                println!(
                    "[draft]   application={} source={} -> Drafted ({})",
                    outcome.application_id, outcome.source, note,
                );
            }
        },
        Err(e) => {
            tracing::error!(error = %format_args!("{e:#}"), "retry failed");
            std::process::exit(crate::commands::apply::map_apply_error_to_exit_code(&e));
        }
    }
    Ok(())
}
