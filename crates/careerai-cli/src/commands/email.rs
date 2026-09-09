//! `careerai email` — draft a cold application email for a listing.

use anyhow::Result;
use careerai_db::queries;
use careerai_pipeline::{build_llm, load_profile, open_pool};

use crate::load_cfg;

/// Run the email-draft generation for a listing.
pub async fn run_email(cwd: &std::path::Path, listing_id: &str) -> Result<()> {
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

    let draft = careerai_tailor::draft_email(&*llm, &profile, &listing, &cfg.llm).await?;

    println!("Subject: {}", draft.subject);
    println!();
    println!("{}", draft.body);
    println!();
    println!("Attachments: {}", draft.attachments.join(", "));

    Ok(())
}
