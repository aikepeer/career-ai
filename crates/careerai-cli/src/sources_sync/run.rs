use std::path::Path;

use anyhow::{anyhow, Context, Result};
use careerai_core::config::CoreConfig;
use careerai_sources::company_sync::{
    load_embedded_seed, sync as run_sync, AtsVendor, CompanyHit, SyncReport,
};

use super::merge::merge_into_local_yaml;

/// Print + optionally apply the sync.
pub async fn run(cwd: &Path, apply: bool) -> Result<()> {
    let cfg = CoreConfig::load(cwd).context("load config")?;
    let seed = load_embedded_seed().context("parse embedded seed_companies.yaml")?;
    let report = run_sync(&cfg, &seed)
        .await
        .map_err(|e| anyhow!("sync failed: {e}"))?;

    print_report(&report, apply);

    if !apply {
        return Ok(());
    }
    let local_path = cwd.join("config").join("local.yaml");
    let merged = merge_into_local_yaml(&local_path, &report)?;
    write_atomic(&local_path, &merged)?;
    println!();
    println!("wrote {}", local_path.display());
    Ok(())
}

pub(crate) fn write_atomic(dest: &Path, contents: &str) -> Result<()> {
    use std::io::Write as _;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create parent {}", parent.display()))?;
    }
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create tempfile in {}", parent.display()))?;
    tmp.write_all(contents.as_bytes())
        .with_context(|| format!("write tempfile for {}", dest.display()))?;
    tmp.flush()
        .with_context(|| format!("flush tempfile for {}", dest.display()))?;
    tmp.persist(dest).map_err(|e| {
        anyhow::anyhow!(
            "persist {} -> {}: {}",
            e.file.path().display(),
            dest.display(),
            e.error
        )
    })?;
    Ok(())
}

fn print_report(report: &SyncReport, apply: bool) {
    let header = if apply {
        "sources sync (applying):"
    } else {
        "sources sync (preview — pass --apply to write):"
    };
    println!("{header}");

    for ats in [AtsVendor::Greenhouse, AtsVendor::Lever, AtsVendor::Ashby] {
        let adds: Vec<&CompanyHit> = report.add.iter().filter(|h| h.ats == ats).collect();
        let keeps: Vec<&CompanyHit> = report.keep.iter().filter(|h| h.ats == ats).collect();
        let removes: Vec<&String> = report
            .remove
            .iter()
            .filter_map(|(a, s)| if *a == ats { Some(s) } else { None })
            .collect();
        if adds.is_empty() && keeps.is_empty() && removes.is_empty() {
            continue;
        }
        println!("  {}:", ats.as_str());
        for hit in &adds {
            println!(
                "    + {}  ({} matches: {})",
                hit.slug,
                hit.matched_jobs,
                hit.matched_domains.join(" + "),
            );
        }
        for hit in &keeps {
            println!(
                "    keep {}  ({} matches: {})",
                hit.slug,
                hit.matched_jobs,
                hit.matched_domains.join(" + "),
            );
        }
        for slug in removes {
            println!("    - {slug}    (no current matches; consider removing)");
        }
    }

    if !report.probe_failures.is_empty() {
        println!();
        println!("  probe failures (soft-failed, sync continued):");
        for (slug, reason) in &report.probe_failures {
            println!("    ! {slug}: {reason}");
        }
    }

    if !apply {
        println!();
        println!("  next: rerun with --apply to write changes to config/local.yaml");
    }
}
