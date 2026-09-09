//! `careerai negotiate` — generate a salary negotiation script for a listing.

use anyhow::Result;
use careerai_db::queries;
use careerai_pipeline::{build_llm, load_profile, open_pool};

use crate::load_cfg;

/// Run the negotiation-script generation for a listing.
pub async fn run_negotiate(
    cwd: &std::path::Path,
    listing_id: &str,
    benchmark: Option<&str>,
) -> Result<()> {
    let cfg = load_cfg(cwd)?;
    let pool = open_pool(cwd).await?;
    let listing = match queries::find_by_id(&pool, listing_id).await {
        Ok(l) => l,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("listing not found: {listing_id}");
        }
        Err(e) => return Err(anyhow::anyhow!(e)),
    };
    let profile = load_profile(cwd)?;
    let llm = build_llm(cwd, &cfg).await?;

    let script = careerai_tailor::generate_negotiation_script(
        &*llm, &profile, &listing, &cfg.llm, benchmark,
    )
    .await?;

    println!(
        "=== Negotiation Script for {} @ {} ===\n",
        listing.title, listing.company
    );
    println!(
        "Suggested counter: {}-{}% above initial offer\n",
        script.counter_percent_low, script.counter_percent_high
    );
    println!("Walk-away threshold: {}\n", script.walkaway_threshold);

    println!(
        "=== Counter-Offer Email ===\n{}\n",
        script.counter_offer_email
    );

    println!("=== Talking Points ===\n");
    for (i, tp) in script.talking_points.iter().enumerate() {
        println!("  [{}] {}", i + 1, tp.topic);
        println!("      {}\n", tp.script);
    }

    Ok(())
}
