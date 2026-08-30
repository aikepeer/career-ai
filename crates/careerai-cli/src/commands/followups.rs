//! `careerai followups` — list submitted applications that have gone
//! quiet (ported from career-ops `followup-cadence.mjs`).
//!
//! An application is a follow-up candidate when its latest `submitted`
//! transition is at least `--days` old and it never received a
//! `responded` transition. Oldest first, so the operator starts with
//! the most overdue.

use std::path::Path;

use anyhow::Result;

use careerai_pipeline as pipeline;

pub async fn run(root: &Path, days: u32) -> Result<()> {
    let items = pipeline::list_followups(root, i64::from(days)).await?;
    if items.is_empty() {
        println!(
            "(no follow-ups due — every submitted application is either \
             fresh or already responded)"
        );
        return Ok(());
    }
    println!("applications quiet for >= {days} days ({}):", items.len());
    for (i, it) in items.iter().enumerate() {
        println!(
            "{:>2}. {} @ {} ({}) — submitted {}d ago\n    id:   {}\n    url:  {}",
            i + 1,
            it.title,
            it.company,
            it.source,
            it.days_since,
            it.listing_id,
            it.url,
        );
    }
    Ok(())
}
