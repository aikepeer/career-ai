//! Cover-letter and resume-bullet reuse library queries (C4).
//!
//! When the LLM generates a new tailored result, the cover letter and
//! bullets are stored by job domain and role. When tailoring a new listing
//! in a known domain, the stored cover letter is reused directly — no LLM
//! call needed for the cover letter draft.

use sqlx::SqlitePool;

use crate::error::Result;

/// Store a cover letter template in the library, indexed by domain, role,
/// and profile_hash. The profile_hash ensures a letter written for one
/// candidate is never reused for a different candidate with the same
/// domain and role (R02).
pub async fn store_cover_letter(
    pool: &SqlitePool,
    domain: &str,
    role: &str,
    template_text: &str,
    source_application_id: &str,
    profile_hash: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cover_letter_library (domain, role, template_text, source_application_id, profile_hash)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(domain)
    .bind(role)
    .bind(template_text)
    .bind(source_application_id)
    .bind(profile_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch the most recently stored cover letter for a domain + role,
/// scoped to the given profile_hash. Returns `None` if no entry exists
/// for this candidate. R02: the profile_hash constraint prevents
/// cross-candidate reuse.
pub async fn fetch_cover_letter_for_domain(
    pool: &SqlitePool,
    domain: &str,
    role: &str,
    profile_hash: &str,
) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT template_text FROM cover_letter_library
         WHERE domain = ? AND role = ? AND profile_hash = ?
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(domain)
    .bind(role)
    .bind(profile_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(text,)| text))
}

/// Store a bullet in the library, indexed by domain + role.
pub async fn store_bullet(
    pool: &SqlitePool,
    domain: &str,
    role: &str,
    bullet_text: &str,
    theme_label: &str,
    source_application_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO bullet_library (domain, role, bullet_text, theme_label, source_application_id)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(domain)
    .bind(role)
    .bind(bullet_text)
    .bind(theme_label)
    .bind(source_application_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch all bullets for a domain + role, ordered by use count descending.
pub async fn fetch_bullets_for_domain(
    pool: &SqlitePool,
    domain: &str,
    role: &str,
) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT bullet_text, theme_label FROM bullet_library
         WHERE domain = ? AND role = ?
         ORDER BY use_count DESC, created_at DESC",
    )
    .bind(domain)
    .bind(role)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Summary stats for the dashboard's content library browser.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LibraryStats {
    pub bullet_count: u64,
    pub cover_letter_count: u64,
    pub domains: Vec<DomainEntry>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DomainEntry {
    pub domain: String,
    pub role: String,
    pub bullets: u64,
    pub cover_letters: u64,
}

/// Aggregate stats for the dashboard.
#[allow(clippy::cast_sign_loss)]
pub async fn library_stats(pool: &SqlitePool) -> Result<LibraryStats> {
    let (bullet_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bullet_library")
        .fetch_one(pool)
        .await?;
    let (cover_letter_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cover_letter_library")
        .fetch_one(pool)
        .await?;

    let bullet_entries: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT domain, role, COUNT(*) as cnt FROM bullet_library
         GROUP BY domain, role ORDER BY cnt DESC",
    )
    .fetch_all(pool)
    .await?;

    let cl_entries: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT domain, role, COUNT(*) as cnt FROM cover_letter_library
         GROUP BY domain, role ORDER BY cnt DESC",
    )
    .fetch_all(pool)
    .await?;

    let mut domains: Vec<DomainEntry> = Vec::new();
    for (dom, rl, cnt) in &bullet_entries {
        domains.push(DomainEntry {
            domain: dom.clone(),
            role: rl.clone(),
            bullets: *cnt as u64,
            cover_letters: 0,
        });
    }
    for (dom, rl, cnt) in &cl_entries {
        if let Some(entry) = domains
            .iter_mut()
            .find(|d| &d.domain == dom && &d.role == rl)
        {
            entry.cover_letters = *cnt as u64;
        } else {
            domains.push(DomainEntry {
                domain: dom.clone(),
                role: rl.clone(),
                bullets: 0,
                cover_letters: *cnt as u64,
            });
        }
    }

    Ok(LibraryStats {
        bullet_count: bullet_count as u64,
        cover_letter_count: cover_letter_count as u64,
        domains,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::applications::create_application;
    use crate::queries::listings::insert_or_ignore;
    use crate::queries::test_support::{fixture, new_app};

    #[tokio::test]
    async fn store_and_fetch_cover_letter() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "cl-1"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        store_cover_letter(
            &pool,
            "ml",
            "engineer",
            "Dear ML team...",
            &app.id,
            "hash-1",
        )
        .await
        .unwrap();

        let result = fetch_cover_letter_for_domain(&pool, "ml", "engineer", "hash-1")
            .await
            .unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Dear ML team...");
    }

    #[tokio::test]
    async fn fetch_cover_letter_returns_none_for_unknown_domain() {
        let pool = pool_in_memory().await.unwrap();
        let result = fetch_cover_letter_for_domain(&pool, "unknown", "role", "hash-1")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn store_and_fetch_bullets() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "bl-1"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        store_bullet(
            &pool,
            "embedded",
            "firmware",
            "Built Yocto BSP",
            "BSP",
            &app.id,
        )
        .await
        .unwrap();
        store_bullet(
            &pool,
            "embedded",
            "firmware",
            "Ported Linux kernel",
            "Kernel",
            &app.id,
        )
        .await
        .unwrap();

        let bullets = fetch_bullets_for_domain(&pool, "embedded", "firmware")
            .await
            .unwrap();
        assert_eq!(bullets.len(), 2);
    }

    #[tokio::test]
    async fn library_stats_aggregates() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "ls-1"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        store_bullet(&pool, "ml", "engineer", "bullet 1", "label", &app.id)
            .await
            .unwrap();
        store_cover_letter(&pool, "ml", "engineer", "letter", &app.id, "hash-1")
            .await
            .unwrap();
        store_bullet(&pool, "embedded", "firmware", "bullet 2", "label2", &app.id)
            .await
            .unwrap();

        let stats = library_stats(&pool).await.unwrap();
        assert_eq!(stats.bullet_count, 2);
        assert_eq!(stats.cover_letter_count, 1);
        assert_eq!(stats.domains.len(), 2);
    }
}
