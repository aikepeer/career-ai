//! `careerai mark-responded` — record an employer response and prepare context.

use std::path::Path;

use anyhow::Result;
use careerai_core::config::CoreConfig;

/// Mark the application as responded and optionally generate its prep sheet.
pub async fn run(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
    note: Option<&str>,
    no_prep: bool,
) -> Result<()> {
    careerai_pipeline::mark_responded(root, application_id, note).await?;
    println!("responded: application_id={application_id}");
    if !no_prep {
        let path = careerai_prep::generate(root, cfg, application_id).await?;
        println!("prep sheet: {}", path.display());
    }
    Ok(())
}
