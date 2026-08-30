//! `careerai liveness` — verify shortlisted listings are still open
//! (ported from career-ops `check-liveness.mjs`).
//!
//! Zero-token: probes the source board's single-posting API endpoint
//! (404 = closed). Aggregator sources without a per-posting API are
//! reported `unknown` and never acted on.

use std::path::Path;

use anyhow::Result;

use careerai_pipeline as pipeline;

pub async fn run(root: &Path, source: Option<&str>) -> Result<()> {
    let rows = pipeline::check_liveness(root, source).await?;
    if rows.is_empty() {
        println!("(no shortlisted listings to check)");
        return Ok(());
    }
    let (mut alive, mut closed, mut unknown) = (0usize, 0usize, 0usize);
    for (i, r) in rows.iter().enumerate() {
        match r.verdict {
            pipeline::Liveness::Alive => alive += 1,
            pipeline::Liveness::Closed => closed += 1,
            pipeline::Liveness::Unknown => unknown += 1,
        }
        println!(
            "{:>2}. [{}] {} @ {} ({})\n    id:   {}\n    url:  {}",
            i + 1,
            r.verdict.label(),
            r.title,
            r.company,
            r.source,
            r.listing_id,
            r.url,
        );
    }
    println!(
        "\nliveness: alive {alive}, closed {closed}, unknown {unknown} \
         (closed postings can be rolled back with `careerai rollback <id> --to discovered`)"
    );
    Ok(())
}
