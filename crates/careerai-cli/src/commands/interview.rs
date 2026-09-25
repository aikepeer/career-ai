//! `careerai interview` — generate an interview prep pack for a listing.

use anyhow::Result;
use careerai_db::queries;
use careerai_pipeline::{build_llm, load_profile, open_pool};

use crate::load_cfg;

/// Run the interview-prep generation for a listing.
pub async fn run_interview(cwd: &std::path::Path, listing_id: &str) -> Result<()> {
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

    println!(
        "Generating interview prep for: {} @ {}",
        listing.title, listing.company
    );
    println!();

    let prep =
        careerai_tailor::generate_interview_prep(&*llm, &profile, &listing, &cfg.llm).await?;

    println!("=== STAR Stories ({}) ===\n", prep.star_stories.len());
    for (i, story) in prep.star_stories.iter().enumerate() {
        println!("  [{}] {}", i, story.source);
        println!("    S: {}", story.situation);
        println!("    T: {}", story.task);
        println!("    A: {}", story.action);
        if !story.result.is_empty() {
            println!("    R: {}", story.result);
        }
        println!();
    }

    println!("=== Likely Questions ({}) ===\n", prep.questions.len());
    for q in &prep.questions {
        println!("  Q: {}", q.question);
        if let Some(idx) = q.best_story_idx {
            if idx < prep.star_stories.len() {
                println!(
                    "    → STAR story #{}: {}",
                    idx, prep.star_stories[idx].source
                );
            }
        }
        if let Some(note) = &q.prep_note {
            println!("    Note: {note}");
        }
        println!();
    }

    println!("=== General Advice ===\n{}\n", prep.general_advice);

    Ok(())
}
