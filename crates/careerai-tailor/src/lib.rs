//! Resume tailoring + cover letter drafting.
//!
//! Safety invariant: the tailoring step emits a constrained JSON diff
//! that can only reorder or rewrite existing bullets. It MUST NOT invent
//! experience, titles, dates, or employers. The validator in `diff.rs`
//! rejects anything outside that grammar; `guardrails.rs` polices
//! entity-level details (numbers, years, proper nouns) on each reword.

#![forbid(unsafe_code)]

pub mod cover_letter;
pub mod diff;
pub mod error;
pub mod guardrails;
pub mod model;
pub mod prompt;
pub mod schema;

use std::path::PathBuf;

use careerai_core::config::LlmConfig;
use careerai_db::models::NewApplication;
use careerai_db::queries;
use careerai_llm::cache::Cache;
use careerai_llm::hashing::{canonical_profile_hash, compose_key, jd_hash};
use careerai_llm::trait_def::Llm;
use careerai_profile::schema::Profile;
use sqlx::SqlitePool;
use tracing::info;

pub use crate::error::{Result, TailorError};
pub use crate::model::{CoverLetter, ExperienceView, ProjectView, ResumeView, TailorOutcome};

/// Tailor a shortlisted listing end-to-end: fetch the listing, call the
/// LLM (cache-wrapped) for a constrained diff, validate + apply it,
/// draft a cover letter, persist application + payload, transition the
/// listing to `Tailored`, return the outcome.
///
/// # Errors
/// See `TailorError`. `InventedContent` / `Schema` indicate the LLM's
/// output violated the safety invariant; `Db` / `Llm` indicate an
/// infrastructure failure.
pub async fn tailor_for_listing(
    pool: &SqlitePool,
    llm: &(dyn Llm + Send + Sync),
    listing_id: &str,
    profile: &Profile,
    cfg: &LlmConfig,
) -> Result<TailorOutcome> {
    // 1) Fetch the listing.
    let listing = queries::find_by_id(pool, listing_id).await?;

    // 2) Build tailor prompt.
    let req = prompt::tailor_prompt(profile, &listing, cfg)?;

    // 3) Cache-wrapped LLM call.
    let cache_root = if cfg.cache_dir.is_empty() {
        PathBuf::from("data/cache/llm")
    } else {
        PathBuf::from(&cfg.cache_dir)
    };
    let cache = Cache::new(cache_root);
    let profile_hash = canonical_profile_hash(profile);
    let jd = jd_hash(&listing.title, &listing.company, &listing.description);
    let key = compose_key(&req.prompt_version, &profile_hash, &jd, &req.model);

    let resp = if let Some(hit) = cache.get(&key).await? {
        info!(
            target: "tailor",
            cache = "hit",
            prompt_version = %req.prompt_version,
            listing_id = %listing.id,
            "tailor cache hit"
        );
        hit
    } else {
        info!(
            target: "tailor",
            cache = "miss",
            prompt_version = %req.prompt_version,
            listing_id = %listing.id,
            "tailor calling llm"
        );
        let fresh = llm.complete(&req).await?;
        cache.put(&key, &fresh).await?;
        fresh
    };

    // 4) Parse + validate the diff; apply it to produce the ResumeView.
    let doc = schema::parse_and_validate(&resp.text, profile)?;
    let diff_raw_json = resp.text.clone();
    let resume_view = diff::apply(doc, profile.clone())?;

    // 5) Draft the cover letter (separate cache scope with its own
    //    prompt_version suffix in `cover_letter::draft`). Pass the
    //    pre-computed profile_hash so we don't re-canonicalize the
    //    whole profile JSON tree.
    let letter = cover_letter::draft(llm, profile, &listing, cfg, &cache, &profile_hash).await?;

    // 6) Persist application row + payload.
    let new_app = NewApplication {
        listing_id: listing.id.clone(),
        profile_hash: profile_hash.clone(),
        prompt_version: req.prompt_version.clone(),
        llm_model: req.model.clone(),
    };
    let app = queries::create_application(pool, &new_app).await?;

    let resume_view_json = serde_json::to_string(&resume_view)?;
    queries::write_payload(
        pool,
        &app.id,
        &resume_view_json,
        &letter.body,
        &diff_raw_json,
    )
    .await?;

    // 7) Transition the listing to Tailored.
    queries::transition(
        pool,
        &listing.id,
        careerai_core::state::ListingState::Tailored,
        Some(&format!("application_id={}", app.id)),
    )
    .await?;

    Ok(TailorOutcome {
        application_id: app.id,
        resume_view,
        cover_letter: letter,
        diff_raw_json,
    })
}
