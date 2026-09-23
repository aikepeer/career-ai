//! Rehydrate profile-scoped compiled variants and cover skeletons from SQLite.

use careerai_db::queries::{list_bullet_variants, list_cover_skeletons};
use sqlx::SqlitePool;

use crate::cover_skeleton::CoverSkeleton;
use crate::error::{Result, TailorError};
use crate::variants::{BulletVariant, EntryVariants, ProfileVariants};

pub async fn load_compiled_material(
    pool: &SqlitePool,
    profile_hash: &str,
) -> Result<(Option<ProfileVariants>, Vec<CoverSkeleton>)> {
    let variant_rows = list_bullet_variants(pool, profile_hash).await?;
    let variants = if variant_rows.is_empty() {
        None
    } else {
        let mut profile = ProfileVariants {
            profile_hash: profile_hash.to_string(),
            ..ProfileVariants::default()
        };
        for row in variant_rows {
            let entry_index = usize::try_from(row.entry_index)
                .map_err(|_| TailorError::Schema("negative variant entry index".into()))?;
            let bullet_index = usize::try_from(row.bullet_index)
                .map_err(|_| TailorError::Schema("negative variant bullet index".into()))?;
            let target = if row.entry_kind == "experience" {
                &mut profile.experience
            } else {
                &mut profile.projects
            };
            while target.len() <= entry_index {
                target.push(EntryVariants {
                    entry_index: target.len(),
                    bullets: Vec::new(),
                });
            }
            let entry = &mut target[entry_index];
            while entry.bullets.len() <= bullet_index {
                entry.bullets.push(BulletVariant {
                    original: String::new(),
                    variants: Vec::new(),
                });
            }
            entry.bullets[bullet_index] = BulletVariant {
                original: row.original,
                variants: serde_json::from_str(&row.variants_json)?,
            };
        }
        Some(profile)
    };

    let skeletons = list_cover_skeletons(pool, profile_hash)
        .await?
        .into_iter()
        .map(|row| {
            Ok(CoverSkeleton {
                domain: row.domain,
                keywords: serde_json::from_str(&row.keywords_json)?,
                template: row.template,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((variants, skeletons))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_db::pool::pool_in_memory;
    use careerai_db::queries::{replace_bullet_variants, NewBulletVariant};

    #[tokio::test]
    async fn loads_variants_without_cross_profile_leakage() {
        let pool = pool_in_memory().await.unwrap();
        replace_bullet_variants(
            &pool,
            "sha256:a",
            &[NewBulletVariant {
                profile_hash: "sha256:a".into(),
                entry_kind: "experience".into(),
                entry_index: 0,
                bullet_index: 0,
                original: "original".into(),
                variants_json: r#"["variant"]"#.into(),
            }],
        )
        .await
        .unwrap();
        let (variants, skeletons) = load_compiled_material(&pool, "sha256:b").await.unwrap();
        assert!(variants.is_none());
        assert!(skeletons.is_empty());
    }
}
