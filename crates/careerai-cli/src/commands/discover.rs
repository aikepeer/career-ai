//! `careerai discover` — pull new listings from configured sources.

use std::path::Path;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

pub async fn run(cwd: &Path, source_filter: &[String]) -> Result<()> {
    let cfg = CoreConfig::load(cwd).context("load config")?;
    let report = pipeline::discover_all(cwd, &cfg, source_filter).await?;
    println!(
        "discover: fetched {}, new {}, duplicates {}, errors {}",
        report.fetched, report.new_rows, report.duplicates, report.errors,
    );
    Ok(())
}
