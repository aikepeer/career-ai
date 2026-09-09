//! JD + profile similarity reuse queries (C3).
//!
//! Before calling the LLM in `tailor_for_listing`, we check whether a
//! *similar* listing with the *same* profile hash has already been
//! tailored. If so, the stored diff/cover-letter/resume-view is reused,
//! skipping the LLM call entirely.
//!
//! Similarity match: same `profile_hash` + same `company` + title Jaccard
//! ≥ 0.6. This catches reposts and near-duplicate listings across sources.

use sqlx::SqlitePool;

use crate::error::{DbError, Result};
use crate::models::ApplicationPayload;

/// A similarity-reuse hit: the application payload from a previously
/// tailored listing, ready to be applied to the new listing.
#[derive(Debug, Clone)]
pub struct SimilarTailoredResult {
    pub source_listing_id: String,
    pub application_id: String,
    pub payload: ApplicationPayload,
    pub title_jaccard: f32,
}

/// Record a successful tailor in the similarity index so future listings
/// can reuse it.
pub async fn record_similarity_index(
    pool: &SqlitePool,
    listing_id: &str,
    profile_hash: &str,
    company: &str,
    title: &str,
    jd_hash: &str,
    application_id: &str,
) -> Result<()> {
    let title_normalized = normalize_title(title);
    sqlx::query(
        "INSERT INTO jd_similarity_index
            (listing_id, profile_hash, company, title, title_normalized, jd_hash, application_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(listing_id)
    .bind(profile_hash)
    .bind(company)
    .bind(title)
    .bind(&title_normalized)
    .bind(jd_hash)
    .bind(application_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find a previously tailored result for a *similar* listing with the
/// same profile hash. Returns the best match (highest title Jaccard)
/// or `None` if no candidate meets the threshold.
pub async fn find_similar_tailored(
    pool: &SqlitePool,
    company: &str,
    title: &str,
    profile_hash: &str,
    exclude_listing_id: &str,
) -> Result<Option<SimilarTailoredResult>> {
    // Step 1: SQL narrows candidates to same profile_hash + same company.
    let candidates: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT s.listing_id, s.application_id, s.title
         FROM jd_similarity_index s
         JOIN application_payloads p ON s.application_id = p.application_id
         WHERE s.profile_hash = ?
           AND s.company = ?
           AND s.listing_id != ?",
    )
    .bind(profile_hash)
    .bind(company)
    .bind(exclude_listing_id)
    .fetch_all(pool)
    .await?;

    if candidates.is_empty() {
        return Ok(None);
    }

    // Step 2: Rust-side Jaccard on title tokens.
    let query_tokens = tokenize_title(title);
    let mut best: Option<(f32, String)> = None;
    for (_src_listing, app_id, stored_title) in &candidates {
        let stored_tokens = tokenize_title(stored_title);
        let sim = jaccard(&query_tokens, &stored_tokens);
        if sim >= 0.6 {
            match &best {
                Some((best_sim, _)) if *best_sim >= sim => {}
                _ => best = Some((sim, app_id.clone())),
            }
        }
    }

    let Some((title_jaccard, app_id)) = best else {
        return Ok(None);
    };

    // Step 3: Fetch the payload for the best match.
    let payload: Option<ApplicationPayload> = sqlx::query_as(
        "SELECT application_id, resume_view_json, cover_letter_text, diff_json, created_at
         FROM application_payloads WHERE application_id = ?",
    )
    .bind(&app_id)
    .fetch_optional(pool)
    .await?;

    let payload = payload.ok_or_else(|| DbError::NotFound(app_id.clone()))?;
    let source_listing_id = candidates
        .iter()
        .find(|(_, a, _)| a == &app_id)
        .map(|(l, _, _)| l.clone())
        .unwrap_or_default();

    Ok(Some(SimilarTailoredResult {
        source_listing_id,
        application_id: app_id,
        payload,
        title_jaccard,
    }))
}

/// Lowercase + keep only alphanumeric tokens, joined by space.
fn normalize_title(title: &str) -> String {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tokenize a title for Jaccard comparison.
fn tokenize_title(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(str::to_lowercase)
        .collect()
}

/// Jaccard similarity between two token lists.
fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&String> = a.iter().collect();
    let set_b: std::collections::HashSet<&String> = b.iter().collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    #[allow(clippy::cast_precision_loss)]
    {
        intersection as f32 / union as f32
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::applications::create_application;
    use crate::queries::listings::insert_or_ignore;
    use crate::queries::payloads::write_payload;
    use crate::queries::test_support::{fixture, new_app};

    #[tokio::test]
    async fn record_and_find_similar_tailored() {
        let pool = pool_in_memory().await.unwrap();

        // Insert listing A (the "already tailored" one).
        let mut listing_a = fixture("greenhouse", "sim-a");
        listing_a.title = "Senior Embedded Linux Engineer".into();
        listing_a.company = "Acme Robotics".into();
        let (listing_a_id, _) = insert_or_ignore(&pool, &listing_a).await.unwrap();

        // Create an application + payload for listing A.
        let app = create_application(&pool, &new_app(&listing_a_id))
            .await
            .unwrap();
        write_payload(
            &pool,
            &app.id,
            r#"{"v":1}"#,
            "cover letter body",
            r#"{"diff":true}"#,
        )
        .await
        .unwrap();

        // Record the similarity index.
        record_similarity_index(
            &pool,
            &listing_a_id,
            "profile-hash-123",
            "Acme Robotics",
            "Senior Embedded Linux Engineer",
            "jd-hash-abc",
            &app.id,
        )
        .await
        .unwrap();

        // Insert listing B (the "new" one — same company, very similar title).
        let mut new_listing = fixture("greenhouse", "sim-b");
        new_listing.title = "Senior Embedded Linux Engineer".into();
        new_listing.company = "Acme Robotics".into();
        let (new_listing_id, _) = insert_or_ignore(&pool, &new_listing).await.unwrap();

        // Find similar tailored result for listing B.
        let result = find_similar_tailored(
            &pool,
            "Acme Robotics",
            "Senior Embedded Linux Engineer",
            "profile-hash-123",
            &new_listing_id,
        )
        .await
        .unwrap();

        assert!(result.is_some(), "should find a similar tailored result");
        let r = result.unwrap();
        assert_eq!(r.application_id, app.id);
        assert!(r.title_jaccard >= 0.6);
        assert_eq!(r.payload.cover_letter_text, "cover letter body");
    }

    #[tokio::test]
    async fn find_similar_returns_none_for_different_company() {
        let pool = pool_in_memory().await.unwrap();

        let mut listing_a = fixture("greenhouse", "sim-c");
        listing_a.title = "Senior ML Engineer".into();
        listing_a.company = "Acme Robotics".into();
        let (listing_a_id, _) = insert_or_ignore(&pool, &listing_a).await.unwrap();

        let app = create_application(&pool, &new_app(&listing_a_id))
            .await
            .unwrap();
        write_payload(&pool, &app.id, r#"{"v":1}"#, "body", r#"{"d":1}"#)
            .await
            .unwrap();

        record_similarity_index(
            &pool,
            &listing_a_id,
            "ph",
            "Acme Robotics",
            "Senior ML Engineer",
            "jd",
            &app.id,
        )
        .await
        .unwrap();

        // Different company — should not match.
        let result = find_similar_tailored(
            &pool,
            "Other Corp",
            "Senior ML Engineer",
            "ph",
            "other-listing",
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn find_similar_returns_none_for_different_profile_hash() {
        let pool = pool_in_memory().await.unwrap();

        let mut listing_a = fixture("greenhouse", "sim-d");
        listing_a.title = "Senior ML Engineer".into();
        listing_a.company = "Acme".into();
        let (listing_a_id, _) = insert_or_ignore(&pool, &listing_a).await.unwrap();

        let app = create_application(&pool, &new_app(&listing_a_id))
            .await
            .unwrap();
        write_payload(&pool, &app.id, r#"{"v":1}"#, "body", r#"{"d":1}"#)
            .await
            .unwrap();

        record_similarity_index(
            &pool,
            &listing_a_id,
            "profile-A",
            "Acme",
            "Senior ML Engineer",
            "jd",
            &app.id,
        )
        .await
        .unwrap();

        // Same company + title, but different profile hash.
        let result =
            find_similar_tailored(&pool, "Acme", "Senior ML Engineer", "profile-B", "other")
                .await
                .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn find_similar_returns_none_for_dissimilar_title() {
        let pool = pool_in_memory().await.unwrap();

        let mut listing_a = fixture("greenhouse", "sim-e");
        listing_a.title = "Senior ML Engineer".into();
        listing_a.company = "Acme".into();
        let (listing_a_id, _) = insert_or_ignore(&pool, &listing_a).await.unwrap();

        let app = create_application(&pool, &new_app(&listing_a_id))
            .await
            .unwrap();
        write_payload(&pool, &app.id, r#"{"v":1}"#, "body", r#"{"d":1}"#)
            .await
            .unwrap();

        record_similarity_index(
            &pool,
            &listing_a_id,
            "ph",
            "Acme",
            "Senior ML Engineer",
            "jd",
            &app.id,
        )
        .await
        .unwrap();

        // Same company, very different title — Jaccard < 0.6.
        let result =
            find_similar_tailored(&pool, "Acme", "Embedded Firmware Developer", "ph", "other")
                .await
                .unwrap();
        assert!(result.is_none());
    }
}
