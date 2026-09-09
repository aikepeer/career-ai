//! `careerai upskill` — skill gap analysis between profile and shortlisted JDs.

use anyhow::Result;
use careerai_db::queries;
use careerai_pipeline::{load_profile, open_pool};
use careerai_sources::RawListing;
use std::path::Path;

/// Run the skill-gap analysis.
pub async fn run_upskill(cwd: &Path) -> Result<()> {
    let pool = open_pool(cwd).await?;
    let profile = load_profile(cwd)?;

    // Pull all shortlisted listings from the DB and convert to RawListing
    // for the analyser.
    let listings =
        queries::list_by_state(&pool, careerai_core::state::ListingState::Shortlisted, 500).await?;
    let raw: Vec<RawListing> = listings
        .iter()
        .map(|l| RawListing {
            source: l.source.clone(),
            external_id: l.external_id.clone(),
            title: l.title.clone(),
            company: l.company.clone(),
            location: l.location.clone(),
            url: l.url.clone(),
            description: l.description.clone(),
            raw_json: None,
        })
        .collect();

    let report = careerai_match::analyse_skill_gap(&profile, &raw);

    println!("=== Skill Gap Analysis ===");
    println!("Shortlisted JDs analysed: {}\n", report.total_jds);

    println!("--- Strengths (skills you have that JDs want) ---\n");
    println!("{:<25} {:>10}", "Skill", "JD Count");
    println!("{}", "-".repeat(37));
    for s in report.strengths.iter().take(20) {
        println!("{:<25} {:>10}", truncate(&s.skill, 25), s.jd_count);
    }

    println!("\n--- Gaps (skills JDs want that you lack) ---\n");
    println!("{:<25} {:>10}", "Skill", "JD Count");
    println!("{}", "-".repeat(37));
    for g in &report.gaps {
        println!("{:<25} {:>10}", truncate(&g.skill, 25), g.jd_count);
    }

    Ok(())
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}
