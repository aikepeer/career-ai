//! `careerai discover` — pull new listings from configured sources.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

pub async fn run(cwd: &Path, source_filter: &[String]) -> Result<()> {
    let cfg = CoreConfig::load(cwd).context("load config")?;
    let report = pipeline::discover_all(cwd, &cfg, source_filter).await?;
    println!("{}", format_discover_report(&report));
    Ok(())
}

/// Human-readable `discover` output: counts, then one line per newly
/// inserted listing so the operator has the id needed for
/// `careerai tailor <id>` without a second query.
fn format_discover_report(report: &pipeline::DiscoveryReport) -> String {
    let mut out = format!(
        "discover: fetched {}, new {}, duplicates {}, errors {}",
        report.fetched, report.new_rows, report.duplicates, report.errors,
    );
    if !report.new.is_empty() {
        out.push_str("\nnew listings:");
        for l in &report.new {
            let _ = write!(
                out,
                "\n  id: {}  {} @ {} ({})",
                l.id, l.title, l.company, l.source,
            );
        }
        out.push_str("\ntailor one with: `careerai tailor <id>`");
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn format_discover_report_lists_new_ids() {
        let mut report = pipeline::DiscoveryReport {
            fetched: 5,
            new_rows: 2,
            duplicates: 3,
            errors: 0,
            ..Default::default()
        };
        report.new.push(pipeline::NewListingRow {
            id: "gh_42".into(),
            title: "ML Engineer".into(),
            company: "Acme".into(),
            source: "greenhouse".into(),
        });
        report.new.push(pipeline::NewListingRow {
            id: "lv_7".into(),
            title: "Firmware Engineer".into(),
            company: "Widgets Inc".into(),
            source: "lever".into(),
        });

        let out = format_discover_report(&report);
        assert!(out.starts_with("discover: fetched 5, new 2, duplicates 3, errors 0"));
        assert!(out.contains("id: gh_42  ML Engineer @ Acme (greenhouse)"));
        assert!(out.contains("id: lv_7  Firmware Engineer @ Widgets Inc (lever)"));
        assert!(out.contains("careerai tailor <id>"));
    }

    #[test]
    fn format_discover_report_omits_new_section_when_empty() {
        let report = pipeline::DiscoveryReport::default();
        let out = format_discover_report(&report);
        assert_eq!(out, "discover: fetched 0, new 0, duplicates 0, errors 0");
        assert!(!out.contains("new listings"));
    }
}
