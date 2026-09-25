//! Deterministic local tailoring engine: score, rank, prune, and slot-fill.
//! All emitted text comes from the profile or validated skeleton cache.

use careerai_db::models::{Listing, NewApplication};
use careerai_db::queries;
use careerai_match::bullet_score::{BulletScorer, JaccardBulletScorer};
use careerai_profile::schema::{Experience, Profile, Project};
use sqlx::SqlitePool;
use tracing::info;

use crate::compiled::load_compiled_material;
use crate::cover_skeleton::{default_skeletons, extract_slots, fill_skeleton, pick_best_skeleton};
use crate::error::Result;
use crate::model::{CoverLetter, ExperienceView, ProjectView, ResumeView, TailorOutcome};
use crate::variants::ProfileVariants;

/// Deterministically tailors a profile to a listing using local bullet scoring.
pub fn tailor_local(
    profile: &Profile,
    listing: &Listing,
    scorer: &dyn BulletScorer,
    drop_threshold: f32,
) -> Result<ResumeView> {
    tailor_local_with_variants(profile, listing, scorer, drop_threshold, None)
}

/// Deterministically tailors a profile using local bullet scoring and pre-computed variants.
pub fn tailor_local_with_variants(
    profile: &Profile,
    listing: &Listing,
    scorer: &dyn BulletScorer,
    drop_threshold: f32,
    variants: Option<&ProfileVariants>,
) -> Result<ResumeView> {
    let jd_text = format!(
        "{} {} {}",
        listing.title, listing.company, listing.description
    );

    let experience = profile
        .experience
        .iter()
        .enumerate()
        .map(|(entry_idx, exp)| {
            tailor_experience_entry(exp, entry_idx, &jd_text, scorer, drop_threshold, variants)
        })
        .collect();

    let projects = profile
        .projects
        .iter()
        .enumerate()
        .filter_map(|(entry_idx, proj)| {
            tailor_project_entry(proj, entry_idx, &jd_text, scorer, drop_threshold, variants)
        })
        .collect();

    Ok(ResumeView {
        personal: profile.personal.clone(),
        summary: profile.summary.clone(),
        skills: profile.skills.clone(),
        experience,
        education: profile.education.clone(),
        projects,
    })
}

fn tailor_experience_entry(
    exp: &Experience,
    entry_idx: usize,
    jd_text: &str,
    scorer: &dyn BulletScorer,
    drop_threshold: f32,
    variants: Option<&ProfileVariants>,
) -> ExperienceView {
    let mut scored_bullets: Vec<(String, f32)> = exp
        .bullets
        .iter()
        .enumerate()
        .map(|(bullet_idx, b)| {
            let candidates = variants.map_or_else(
                || vec![b.clone()],
                |v| v.experience_bullet_candidates(entry_idx, bullet_idx, b),
            );
            candidates
                .into_iter()
                .map(|c| {
                    let s = scorer.score(&c, jd_text);
                    (c, s)
                })
                .max_by(|a, b_cand| {
                    a.1.partial_cmp(&b_cand.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or_else(|| (b.clone(), 0.0))
        })
        .collect();

    scored_bullets.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut kept_bullets: Vec<String> = scored_bullets
        .iter()
        .filter(|(_, score)| *score >= drop_threshold)
        .map(|(b, _)| b.clone())
        .collect();

    if kept_bullets.is_empty() {
        if let Some((best, _)) = scored_bullets.first() {
            kept_bullets.push(best.clone());
        }
    }

    ExperienceView {
        title: exp.title.clone(),
        company: exp.company.clone(),
        location: if exp.location.is_empty() {
            None
        } else {
            Some(exp.location.clone())
        },
        start: exp.start.clone(),
        end: exp.end.clone(),
        bullets: kept_bullets,
    }
}

fn tailor_project_entry(
    proj: &Project,
    entry_idx: usize,
    jd_text: &str,
    scorer: &dyn BulletScorer,
    drop_threshold: f32,
    variants: Option<&ProfileVariants>,
) -> Option<ProjectView> {
    let mut scored_bullets: Vec<(String, f32)> = proj
        .bullets
        .iter()
        .enumerate()
        .map(|(bullet_idx, b)| {
            let candidates = variants.map_or_else(
                || vec![b.clone()],
                |v| v.project_bullet_candidates(entry_idx, bullet_idx, b),
            );
            candidates
                .into_iter()
                .map(|c| {
                    let s = scorer.score(&c, jd_text);
                    (c, s)
                })
                .max_by(|a, b_cand| {
                    a.1.partial_cmp(&b_cand.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or_else(|| (b.clone(), 0.0))
        })
        .collect();

    scored_bullets.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let kept_bullets: Vec<String> = scored_bullets
        .into_iter()
        .filter(|(_, score)| *score >= drop_threshold)
        .map(|(b, _)| b)
        .collect();

    if kept_bullets.is_empty() {
        return None;
    }

    Some(ProjectView {
        name: proj.name.clone(),
        url: if proj.url.is_empty() {
            None
        } else {
            Some(proj.url.clone())
        },
        bullets: kept_bullets,
    })
}

/// Tailors a listing end-to-end using local deterministic heuristics and persists to SQLite.
pub async fn tailor_for_listing_local(
    pool: &SqlitePool,
    listing_id: &str,
    profile: &Profile,
    drop_threshold: f32,
) -> Result<TailorOutcome> {
    let listing = queries::find_by_id(pool, listing_id).await?;
    let scorer = JaccardBulletScorer;

    info!(
        target: "tailor",
        listing_id = %listing.id,
        drop_threshold,
        "running local deterministic tailor"
    );

    let profile_hash = careerai_llm::hashing::canonical_profile_hash(profile);
    let (compiled_variants, compiled_skeletons) =
        load_compiled_material(pool, &profile_hash).await?;
    let resume_view = crate::local::tailor_local_with_variants(
        profile,
        &listing,
        &scorer,
        drop_threshold,
        compiled_variants.as_ref(),
    )?;
    let diff_raw_json = serde_json::json!({
        "strategy": "local_rank_prune_skeleton",
        "drop_threshold": drop_threshold,
        "compiled_profile_material": compiled_variants.is_some() || !compiled_skeletons.is_empty(),
    })
    .to_string();

    // Prefer validated profile-scoped skeletons; built-ins remain the safe
    // cold-start fallback when compilation has not been run.
    let skeletons = if compiled_skeletons.is_empty() {
        default_skeletons()
    } else {
        compiled_skeletons
    };
    let jd_text = format!(
        "{} {} {}",
        listing.title, listing.company, listing.description
    );
    let best_skeleton = pick_best_skeleton(&skeletons, &jd_text, &scorer);
    let slots = extract_slots(profile, &listing);
    let cover_letter_body = fill_skeleton(best_skeleton, &slots);

    let cover_letter = CoverLetter {
        body: cover_letter_body,
    };

    // Detect tone from company + description.
    let tone_analysis = crate::tone::detect_tone(&listing.company, &listing.description);
    let tone_label_str = crate::tone::tone_label(tone_analysis.tone).to_string();

    let new_app = NewApplication {
        listing_id: listing.id.clone(),
        profile_hash,
        prompt_version: "local.v1".to_string(),
        llm_model: "local".to_string(),
    };
    let app = queries::create_application(pool, &new_app).await?;

    let resume_view_json = serde_json::to_string(&resume_view)?;
    queries::write_payload(
        pool,
        &app.id,
        &resume_view_json,
        &cover_letter.body,
        &diff_raw_json,
    )
    .await?;

    // Compute application quality score.
    let quality_score =
        crate::quality::score_application_quality(&resume_view, &jd_text, &cover_letter.body);
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
        cover_letter,
        diff_raw_json,
        quality_score: Some(quality_score),
        tone_label: Some(tone_label_str),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::Personal;

    fn sample_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Alice Engineer".into(),
                email: "alice@example.com".into(),
                ..Default::default()
            },
            experience: vec![Experience {
                title: "Software Engineer".into(),
                company: "Tech Corp".into(),
                bullets: vec![
                    "Built high throughput Rust microservices with Tokio.".into(),
                    "Maintained legacy COBOL database queries.".into(),
                    "Implemented distributed tracing and Prometheus metrics.".into(),
                ],
                ..Default::default()
            }],
            projects: vec![Project {
                name: "Rust Embedded Kernel".into(),
                bullets: vec!["Wrote custom ARM bootloader in Rust.".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn sample_listing(desc: &str) -> Listing {
        Listing {
            id: "list-1".into(),
            source: "greenhouse".into(),
            external_id: "ext-1".into(),
            title: "Senior Rust Systems Engineer".into(),
            company: "RoboTech".into(),
            location: Some("Remote".into()),
            url: "https://example.com/job".into(),
            description: desc.into(),
            raw_json: None,
            state: "shortlisted".into(),
            score: Some(0.9),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn local_tailor_ranks_relevant_bullets_first() {
        let profile = sample_profile();
        let listing = sample_listing("We need Rust and Tokio microservices with high throughput.");
        let scorer = JaccardBulletScorer;

        let res = tailor_local(&profile, &listing, &scorer, 0.0).unwrap();
        assert_eq!(res.experience.len(), 1);
        let exp = &res.experience[0];
        assert_eq!(exp.bullets.len(), 3);
        assert!(exp.bullets[0].contains("Tokio"));
    }

    #[test]
    fn local_tailor_drops_unrelated_bullets_above_threshold() {
        let profile = sample_profile();
        let listing = sample_listing("Rust Tokio systems engineer.");
        let scorer = JaccardBulletScorer;

        let res = tailor_local(&profile, &listing, &scorer, 0.05).unwrap();
        let exp = &res.experience[0];
        assert!(!exp.bullets.iter().any(|b| b.contains("COBOL")));
    }

    #[test]
    fn local_tailor_retains_at_least_one_bullet_when_all_below_threshold() {
        let profile = sample_profile();
        let listing = sample_listing("Quantum physics and deep sea biology.");
        let scorer = JaccardBulletScorer;

        let res = tailor_local(&profile, &listing, &scorer, 0.5).unwrap();
        let exp = &res.experience[0];
        assert_eq!(exp.bullets.len(), 1, "must retain at least 1 bullet");
    }
}
