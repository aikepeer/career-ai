//! Queries for profile-scoped compiled tailoring content.

use sqlx::SqlitePool;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBulletVariant {
    pub profile_hash: String,
    pub entry_kind: String,
    pub entry_index: i64,
    pub bullet_index: i64,
    pub original: String,
    pub variants_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct BulletVariantRow {
    pub profile_hash: String,
    pub entry_kind: String,
    pub entry_index: i64,
    pub bullet_index: i64,
    pub original: String,
    pub variants_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCoverSkeleton {
    pub profile_hash: String,
    pub domain: String,
    pub keywords_json: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CoverSkeletonRow {
    pub profile_hash: String,
    pub domain: String,
    pub keywords_json: String,
    pub template: String,
}

pub async fn replace_bullet_variants(
    pool: &SqlitePool,
    profile_hash: &str,
    rows: &[NewBulletVariant],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM bullet_variants WHERE profile_hash = ?")
        .bind(profile_hash)
        .execute(&mut *tx)
        .await?;
    for row in rows {
        sqlx::query(
            "INSERT INTO bullet_variants
                (profile_hash, entry_kind, entry_index, bullet_index, original, variants_json)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.profile_hash)
        .bind(&row.entry_kind)
        .bind(row.entry_index)
        .bind(row.bullet_index)
        .bind(&row.original)
        .bind(&row.variants_json)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn list_bullet_variants(
    pool: &SqlitePool,
    profile_hash: &str,
) -> Result<Vec<BulletVariantRow>> {
    Ok(sqlx::query_as(
        "SELECT profile_hash, entry_kind, entry_index, bullet_index, original, variants_json
         FROM bullet_variants
         WHERE profile_hash = ?
         ORDER BY entry_kind, entry_index, bullet_index",
    )
    .bind(profile_hash)
    .fetch_all(pool)
    .await?)
}

pub async fn replace_cover_skeletons(
    pool: &SqlitePool,
    profile_hash: &str,
    rows: &[NewCoverSkeleton],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM cover_skeletons WHERE profile_hash = ?")
        .bind(profile_hash)
        .execute(&mut *tx)
        .await?;
    for row in rows {
        sqlx::query(
            "INSERT INTO cover_skeletons (profile_hash, domain, keywords_json, template)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&row.profile_hash)
        .bind(&row.domain)
        .bind(&row.keywords_json)
        .bind(&row.template)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn list_cover_skeletons(
    pool: &SqlitePool,
    profile_hash: &str,
) -> Result<Vec<CoverSkeletonRow>> {
    Ok(sqlx::query_as(
        "SELECT profile_hash, domain, keywords_json, template
         FROM cover_skeletons
         WHERE profile_hash = ?
         ORDER BY domain",
    )
    .bind(profile_hash)
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;

    #[tokio::test]
    async fn compiled_variants_round_trip_by_profile_hash() {
        let pool = pool_in_memory().await.unwrap();
        let row = NewBulletVariant {
            profile_hash: "sha256:test".into(),
            entry_kind: "experience".into(),
            entry_index: 0,
            bullet_index: 1,
            original: "Built Rust services".into(),
            variants_json: r#"["Built reliable Rust services"]"#.into(),
        };
        replace_bullet_variants(&pool, "sha256:test", std::slice::from_ref(&row))
            .await
            .unwrap();
        let rows = list_bullet_variants(&pool, "sha256:test").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].original, row.original);
        assert_eq!(rows[0].variants_json, row.variants_json);
    }
}
