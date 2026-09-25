//! `careerai patterns` — pattern analysis ported from career-ops
//! `analyze-patterns.mjs` and `detect-reposts.mjs`.
//!
//! Surfaces reposts/ghost-jobs, per-source funnel velocity, advance rates,
//! and rejection latencies from the existing `events` + `listings` tables.

use anyhow::Result;
use careerai_db::queries::patterns::{AdvanceRate, FunnelVelocity, RejectionLatency, Repost};
use std::path::Path;

use careerai_pipeline::open_pool;

/// Run the full pattern analysis and print a human-readable report.
pub async fn run_patterns(cwd: &Path) -> Result<()> {
    let pool = open_pool(cwd).await?;
    let report = careerai_pipeline::analyze_patterns(&pool).await?;

    print_reposts(&report.reposts);
    print_funnel(&report.funnel);
    print_advance_rates(&report.advance_rates);
    print_rejections(&report.rejections);

    if report.reposts.is_empty()
        && report.funnel.is_empty()
        && report.advance_rates.is_empty()
        && report.rejections.is_empty()
    {
        println!("\nNo pipeline data yet — run `careerai discover` and `careerai match` first.");
    }

    Ok(())
}

fn print_reposts(reposts: &[Repost]) {
    if reposts.is_empty() {
        return;
    }
    println!("\n=== Repost / Ghost-Job Detection ===\n");
    println!(
        "{:<30} {:<30} {:<12} {:>5}",
        "Company", "Title", "Source", "Count"
    );
    println!("{}", "-".repeat(80));
    for r in reposts {
        println!(
            "{:<30} {:<30} {:<12} {:>5}",
            truncate(&r.company, 30),
            truncate(&r.title, 30),
            r.source,
            r.occurrences,
        );
    }
}

fn print_funnel(funnel: &[FunnelVelocity]) {
    if funnel.is_empty() {
        return;
    }
    println!("\n=== Funnel Velocity (per source) ===\n");
    println!(
        "{:<12} {:>10} {:>12} {:>10} {:>10} {:>10}",
        "Source", "Discovered", "Shortlisted", "Submitted", "Responded", "Rejected"
    );
    println!("{}", "-".repeat(70));
    for f in funnel {
        println!(
            "{:<12} {:>10} {:>12} {:>10} {:>10} {:>10}",
            f.source, f.discovered, f.shortlisted, f.submitted, f.responded, f.rejected,
        );
    }
}

fn print_advance_rates(rates: &[AdvanceRate]) {
    if rates.is_empty() {
        return;
    }
    println!("\n=== Advance Rates (submitted → responded) ===\n");
    println!(
        "{:<12} {:>10} {:>10} {:>8}",
        "Source", "Submitted", "Responded", "Rate"
    );
    println!("{}", "-".repeat(44));
    for r in rates {
        println!(
            "{:<12} {:>10} {:>10} {:>7.1}%",
            r.source,
            r.submitted,
            r.responded,
            r.rate * 100.0,
        );
    }
}

fn print_rejections(rejections: &[RejectionLatency]) {
    if rejections.is_empty() {
        return;
    }
    println!("\n=== Rejection Latency (slowest first = ghosted then rejected) ===\n");
    println!(
        "{:<25} {:<25} {:<12} {:>10}",
        "Company", "Title", "Source", "Days"
    );
    println!("{}", "-".repeat(76));
    for r in rejections {
        println!(
            "{:<25} {:<25} {:<12} {:>10.1}",
            truncate(&r.company, 25),
            truncate(&r.title, 25),
            r.source,
            r.days_to_reject,
        );
    }
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}
