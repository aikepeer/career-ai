//! `careerai run` — drive the full pipeline in one shot.

use std::path::Path;

use anyhow::Result;

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

/// Run match → tailor → render → apply and print a compact summary.
pub async fn run(root: &Path, cfg: &CoreConfig, auto_submit: bool) -> Result<()> {
    let report = pipeline::run_pipeline(root, cfg, auto_submit).await?;

    println!(
        "run: shortlisted {}, tailored {}, rendered {}",
        report.shortlisted, report.tailored, report.rendered,
    );
    println!(
        "apply: submitted {}, drafted {}, dry_run {}, skipped {}, failed {}",
        report.apply.submitted,
        report.apply.drafted,
        report.apply.dry_run,
        report.apply.skipped,
        report.apply.failed,
    );

    for failure in &report.failures {
        println!(
            "  failed {} {} (source: {}): {}",
            failure.stage,
            failure.id,
            failure.source.as_deref().unwrap_or("unknown"),
            failure.error,
        );
    }

    Ok(())
}
