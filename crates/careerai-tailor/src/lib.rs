//! Resume tailoring + cover letter drafting.
//!
//! Safety invariant: the tailoring step emits a constrained JSON diff
//! that can only reorder or rewrite existing bullets. It MUST NOT invent
//! experience, titles, dates, or employers. The validator in `diff.rs`
//! rejects anything outside that grammar; `guardrails.rs` polices
//! entity-level details (numbers, years, proper nouns) on each reword.

#![forbid(unsafe_code)]

pub mod content_reuse;
pub mod cover_letter;
pub mod cover_skeleton;
pub mod diff;
pub mod email_draft;
pub mod error;
pub mod guardrails;
pub mod interview;
pub mod local;
pub mod model;
pub mod negotiation;
pub mod prompt;
pub mod quality;
pub mod reduce;
pub mod reviewer;
pub mod schema;
pub mod tone;
pub mod variants;

use std::path::{Path, PathBuf};

use careerai_core::config::LlmConfig;
use careerai_db::models::NewApplication;
use careerai_db::queries;
use careerai_llm::cache::{Cache, CacheKey};
use careerai_llm::hashing::{canonical_profile_hash, compose_key, jd_hash};
use careerai_llm::trait_def::Llm;
use careerai_profile::schema::Profile;
use sqlx::SqlitePool;
use tracing::{info, warn};

pub use crate::cover_skeleton::{CoverSkeleton, CoverSlots};
pub use crate::email_draft::{draft_email, email_prompt, EmailDraft};
pub use crate::error::{Result, TailorError};
pub use crate::interview::{
    extract_star_stories, generate_interview_prep, InterviewPrep, InterviewQuestion, StarStory,
};
pub use crate::local::{tailor_for_listing_local, tailor_local, tailor_local_with_variants};
pub use crate::model::{CoverLetter, ExperienceView, ProjectView, ResumeView, TailorOutcome};
pub use crate::negotiation::{
    generate_negotiation_script, negotiation_prompt, NegotiationScript, TalkingPoint,
};
pub use crate::quality::{score_application_quality, QualityScore};
pub use crate::reviewer::{review_tailored, BulletSuggestion, ReviewCritique};
pub use crate::tone::{detect_tone, tone_label, CoverLetterTone, ToneAnalysis};
pub use crate::variants::{
    compile_profile_variants, BulletVariant, EntryVariants, ProfileVariants,
};

/// Resolve the LLM response cache directory, anchoring relative paths to `base_dir`.
fn resolve_cache_root(cfg: &LlmConfig, base_dir: &Path) -> PathBuf {
    if cfg.cache_dir.is_empty() {
        base_dir.join("data").join("cache").join("llm")
    } else {
        let p = PathBuf::from(&cfg.cache_dir);
        if p.is_absolute() {
            p
        } else {
            base_dir.join(p)
        }
    }
}

/// Fetch the tailor LLM response via the priority chain:
/// (1) similarity-index reuse, (2) on-disk cache, (3) fresh LLM call.
/// Returns `(response, from_cache)`.
async fn fetch_tailor_response(
    pool: &SqlitePool,
    llm: &(dyn Llm + Send + Sync),
    req: &careerai_llm::types::LlmRequest,
    listing: &careerai_db::models::Listing,
    cache: &Cache,
    key: &CacheKey,
    profile_hash: &str,
) -> Result<(careerai_llm::types::LlmResponse, bool)> {
    if let Some(sim) = queries::find_similar_tailored(
        pool,
        &listing.company,
        &listing.title,
        profile_hash,
        &listing.id,
    )
    .await?
    {
        info!(
            target: "tailor",
            reuse = "similar",
            listing_id = %listing.id,
            source_listing_id = %sim.source_listing_id,
            jaccard = %sim.title_jaccard,
            "tailor similarity reuse hit — skipping LLM call"
        );
        return Ok((
            careerai_llm::types::LlmResponse {
                text: sim.payload.diff_json,
                prompt_tokens: 0,
                completion_tokens: 0,
                cache_hit: true,
                cached_prompt_tokens: 0,
            },
            true,
        ));
    }

    if let Some(hit) = cache.get(key).await? {
        info!(target: "tailor", cache = "hit", listing_id = %listing.id, "tailor cache hit");
        return Ok((hit, true));
    }

    info!(target: "tailor", cache = "miss", listing_id = %listing.id, "tailor calling llm");
    let fresh = llm.complete(req).await?;
    Ok((fresh, false))
}

/// Draft a cover letter using the priority chain:
/// (a) unified diff body >= 40 chars, (b) content library reuse, (c) LLM draft.
#[allow(clippy::too_many_arguments)]
async fn draft_cover_letter_smart(
    pool: &SqlitePool,
    llm: &(dyn Llm + Send + Sync),
    cover_letter_body: &str,
    listing: &careerai_db::models::Listing,
    profile: &Profile,
    cfg: &LlmConfig,
    cache: &Cache,
    profile_hash: &str,
) -> Result<CoverLetter> {
    if cover_letter_body.trim().len() >= 40 {
        return Ok(CoverLetter {
            body: cover_letter_body.to_string(),
        });
    }

    let (domain, role) = content_reuse::classify_domain(&listing.title);
    match content_reuse::try_reuse_cover_letter(pool, &domain, &role, profile_hash).await {
        Ok(Some(library_letter)) => {
            info!(target: "tailor", reuse = "library", domain = %domain, role = %role, "cover letter library hit");
            return Ok(CoverLetter {
                body: library_letter,
            });
        }
        Ok(None) => {}
        Err(e) => {
            // R02: don't swallow DB errors — log and fall through to the
            // LLM draft rather than silently degrading. The LLM call is
            // the authoritative path; the library is an optimization.
            warn!(target: "tailor", error = %e, "cover letter library fetch failed; falling through to LLM draft");
        }
    }

    cover_letter::draft(llm, profile, listing, cfg, cache, profile_hash).await
}

/// Tailor a shortlisted listing end-to-end: fetch the listing, call the
/// LLM (cache-wrapped) for a constrained diff, validate + apply it,
/// draft a cover letter, persist application + payload, transition the
/// listing to `Tailored`, return the outcome.
///
/// # Errors
/// See `TailorError`. `InventedContent` / `Schema` indicate the LLM's
/// output violated the safety invariant; `Db` / `Llm` indicate an
/// infrastructure failure.
///
/// `base_dir` is the resolved project root: relative `cache_dir` values
/// (e.g. `default.yaml`'s `"data/cache/llm"`) are anchored to it so the
/// response cache is CWD-independent.
#[allow(clippy::too_many_lines)] // quality score + tone detection add ~15 lines
pub async fn tailor_for_listing(
    pool: &SqlitePool,
    llm: &(dyn Llm + Send + Sync),
    listing_id: &str,
    profile: &Profile,
    cfg: &LlmConfig,
    base_dir: &Path,
) -> Result<TailorOutcome> {
    let listing = queries::find_by_id(pool, listing_id).await?;
    let req = prompt::tailor_prompt(profile, &listing, cfg)?;

    let cache = Cache::new(resolve_cache_root(cfg, base_dir));
    let profile_hash = canonical_profile_hash(profile);
    let jd = jd_hash(&listing.title, &listing.company, &listing.description);
    let key = compose_key(&req.prompt_version, &profile_hash, &jd, &req.model);

    let (resp, from_cache) =
        fetch_tailor_response(pool, llm, &req, &listing, &cache, &key, &profile_hash).await?;

    let jd_text = format!(
        "{} {} {}",
        listing.title, listing.company, listing.description
    );
    let doc = schema::parse_and_validate(&resp.text, profile, &jd_text)?;
    let cover_letter_body = doc.cover_letter.clone();
    let diff_raw_json = resp.text.clone();
    let resume_view = diff::apply(doc, profile.clone(), &jd_text)?;

    let (resume_view, reduce_report) = reduce::reduce_view(
        resume_view,
        &jd_text,
        &cover_letter_body,
        &reduce::ReduceConfig::default(),
    );
    if !reduce_report.cut.is_empty() {
        info!(target: "tailor", listing_id = %listing.id, cut = reduce_report.cut.len(), "deterministic reduction trimmed over-budget bullets");
    }

    if !from_cache {
        cache.put(&key, &resp).await?;
    }

    let letter = draft_cover_letter_smart(
        pool,
        llm,
        &cover_letter_body,
        &listing,
        profile,
        cfg,
        &cache,
        &profile_hash,
    )
    .await?;

    // Detect tone from company + description for observability.
    let tone_analysis = detect_tone(&listing.company, &listing.description);
    let tone_label_str = tone_label(tone_analysis.tone).to_string();

    let app = queries::create_application(
        pool,
        &NewApplication {
            listing_id: listing.id.clone(),
            profile_hash: profile_hash.clone(),
            prompt_version: req.prompt_version.clone(),
            llm_model: req.model.clone(),
        },
    )
    .await?;

    let resume_view_json = serde_json::to_string(&resume_view)?;
    queries::write_payload(
        pool,
        &app.id,
        &resume_view_json,
        &letter.body,
        &diff_raw_json,
    )
    .await?;

    let _ = queries::record_similarity_index(
        pool,
        &listing.id,
        &profile_hash,
        &listing.company,
        &listing.title,
        &jd,
        &app.id,
    )
    .await;
    let (domain, role) = content_reuse::classify_domain(&listing.title);
    let _ = queries::store_cover_letter(pool, &domain, &role, &letter.body, &app.id, &profile_hash).await;

    // Compute application quality score from the tailored view, JD text,
    // and cover letter body.
    let quality_score = score_application_quality(&resume_view, &jd_text, &letter.body);
    let recommendations_json =
        serde_json::to_string(&quality_score.recommendations).unwrap_or_else(|_| "[]".to_string());
    let _ = queries::store_quality_score(
        pool,
        &app.id,
        quality_score.overall,
        quality_score.jd_relevance,
        quality_score.skill_coverage,
        quality_score.cover_letter_depth,
        quality_score.bullet_density,
        &recommendations_json,
    )
    .await;

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
        quality_score: Some(quality_score),
        tone_label: Some(tone_label_str),
    })
}
