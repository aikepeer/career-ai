//! Live profile compilation for reusable tailoring material.

use std::path::Path;

use anyhow::{Context, Result};
use careerai_core::config::{BackendChoice, CoreConfig};
use careerai_db::queries::{NewBulletVariant, NewCoverSkeleton};
use careerai_llm::hashing::canonical_profile_hash;

pub async fn run(
    root: &Path,
    cfg: &CoreConfig,
    force: bool,
    backend_override: Option<BackendChoice>,
) -> Result<()> {
    let profile_path = careerai_core::paths::profile_path(root);
    let text = std::fs::read_to_string(&profile_path)
        .with_context(|| format!("read {}", profile_path.display()))?;
    let profile = careerai_profile::Profile::from_yaml(&text).context("parse profile")?;
    let profile_hash = canonical_profile_hash(&profile);
    let cache_dir = careerai_core::paths::cache_dir_for_root(root).join("variants");
    let variants_file = cache_dir.join(format!("{profile_hash}.json"));
    let skeletons_file = cache_dir.join(format!("{profile_hash}.skeletons.json"));
    if variants_file.exists() && skeletons_file.exists() && !force {
        println!(
            "Compiled profile material already exists at {}",
            cache_dir.display()
        );
        println!("Pass --force to re-generate.");
        return Ok(());
    }

    let mut live_cfg = cfg.clone();
    if let Some(backend) = backend_override {
        live_cfg.llm.backend = backend;
    }
    let llm = careerai_pipeline::build_live_llm(root, &live_cfg)
        .await?
        .ok_or_else(|| anyhow::anyhow!("profile compilation requires CAREERAI_LLM_LIVE=1"))?;

    println!("Compiling reusable profile material (hash={profile_hash})...");
    let variants = careerai_tailor::compile_profile_variants(&profile, llm.as_ref(), &live_cfg.llm)
        .await
        .context("compile bullet variants")?;
    let skeletons = careerai_tailor::compile_skeletons(&profile, llm.as_ref(), &live_cfg.llm)
        .await
        .context("compile cover-letter skeletons")?;

    variants.save_to_file(&variants_file)?;
    if let Some(parent) = skeletons_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&skeletons_file, serde_json::to_string_pretty(&skeletons)?)?;

    let pool = careerai_db::pool_from_path(&careerai_core::paths::database_path(root)).await?;
    let bullet_rows = variant_rows(&variants)?;
    careerai_db::queries::replace_bullet_variants(&pool, &profile_hash, &bullet_rows).await?;
    let skeleton_rows = skeletons
        .iter()
        .map(|skeleton| {
            Ok(NewCoverSkeleton {
                profile_hash: profile_hash.clone(),
                domain: skeleton.domain.clone(),
                keywords_json: serde_json::to_string(&skeleton.keywords)?,
                template: skeleton.template.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    careerai_db::queries::replace_cover_skeletons(&pool, &profile_hash, &skeleton_rows).await?;

    println!(
        "Saved {} bullet entries and {} cover skeletons to {}",
        bullet_rows.len(),
        skeleton_rows.len(),
        cache_dir.display()
    );
    Ok(())
}

fn variant_rows(variants: &careerai_tailor::ProfileVariants) -> Result<Vec<NewBulletVariant>> {
    let mut rows = Vec::new();
    for (entry_index, entry) in variants.experience.iter().enumerate() {
        for (bullet_index, bullet) in entry.bullets.iter().enumerate() {
            rows.push(NewBulletVariant {
                profile_hash: variants.profile_hash.clone(),
                entry_kind: "experience".into(),
                entry_index: i64::try_from(entry_index)?,
                bullet_index: i64::try_from(bullet_index)?,
                original: bullet.original.clone(),
                variants_json: serde_json::to_string(&bullet.variants)?,
            });
        }
    }
    for (entry_index, entry) in variants.projects.iter().enumerate() {
        for (bullet_index, bullet) in entry.bullets.iter().enumerate() {
            rows.push(NewBulletVariant {
                profile_hash: variants.profile_hash.clone(),
                entry_kind: "projects".into(),
                entry_index: i64::try_from(entry_index)?,
                bullet_index: i64::try_from(bullet_index)?,
                original: bullet.original.clone(),
                variants_json: serde_json::to_string(&bullet.variants)?,
            });
        }
    }
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn variant_rows_keep_experience_and_project_kinds() {
        let variants = careerai_tailor::ProfileVariants {
            profile_hash: "sha256:test".into(),
            experience: vec![careerai_tailor::EntryVariants {
                entry_index: 0,
                bullets: vec![careerai_tailor::BulletVariant {
                    original: "one".into(),
                    variants: vec!["one".into()],
                }],
            }],
            projects: vec![careerai_tailor::EntryVariants {
                entry_index: 0,
                bullets: vec![careerai_tailor::BulletVariant {
                    original: "two".into(),
                    variants: vec!["two".into()],
                }],
            }],
        };
        let rows = variant_rows(&variants).unwrap();
        assert_eq!(rows[0].entry_kind, "experience");
        assert_eq!(rows[1].entry_kind, "projects");
    }
}
