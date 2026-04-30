//! `careerai match` — run filters + scoring against discovered listings.
//! `--tune` prints the score histogram instead of persisting matches.

use std::path::Path;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

pub async fn run(cwd: &Path, tune: bool) -> Result<()> {
    let cfg = CoreConfig::load(cwd).context("load config")?;
    let report = pipeline::match_all(cwd, &cfg, tune).await?;
    if tune {
        println!(
            "match --tune: {} listings after filters (filtered_out: {})",
            report.histogram.iter().map(|(_, c)| c).sum::<usize>(),
            report.filtered_out,
        );
        println!("score distribution:");
        for (lower, count) in report.histogram {
            let bar = "#".repeat(count.min(60));
            println!("  [{:.1}-{:.1}) {:>4} {bar}", lower, lower + 0.1, count);
        }
        println!(
            "threshold in config: {:.2} — use match (without --tune) to apply",
            cfg.matching.score_threshold,
        );
    } else {
        println!(
            "match: filtered_out {}, shortlisted {}, below-threshold {}",
            report.filtered_out, report.shortlisted, report.also_filtered,
        );
    }
    Ok(())
}
