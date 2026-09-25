//! Hybrid tailoring orchestration for high-confidence JD clusters.

use std::path::Path;

use anyhow::{Context, Result};
use careerai_core::config::CoreConfig;
use careerai_db::queries;
use careerai_match::cluster_jds;

use crate::tailor::{tailor_one_with_pool, TailoredOutcome};

pub async fn tailor_all_clustered(
    root: &Path,
    cfg: &CoreConfig,
    limit: Option<usize>,
) -> Result<Vec<TailoredOutcome>> {
    let pool = crate::open_pool(root).await?;
    let mut query = sqlx::QueryBuilder::new(
        "SELECT id FROM listings WHERE state = 'shortlisted' ORDER BY score DESC",
    );
    if let Some(limit) = limit {
        query
            .push(" LIMIT ")
            .push_bind(i64::try_from(limit).unwrap_or(i64::MAX));
    }
    let ids: Vec<(String,)> = query
        .build_query_as()
        .fetch_all(&pool)
        .await
        .context("fetch clustered shortlisted listings")?;
    let mut listings = Vec::with_capacity(ids.len());
    for (id,) in ids {
        listings.push(queries::find_by_id(&pool, &id).await?);
    }
    let clusters = cluster_jds(&listings, cfg.cluster.threshold);
    tracing::info!(
        clusters = clusters.len(),
        listings = listings.len(),
        "tailoring JD clusters"
    );
    let profile = crate::load_profile(root)?;
    let mut outcomes = Vec::new();

    for cluster in clusters {
        let representative = cluster.representative_index;
        let mut representative_succeeded = false;
        match tailor_one_with_pool(&pool, root, cfg, &listings[representative].id).await {
            Ok(outcome) => {
                outcomes.push(outcome);
                representative_succeeded = true;
            }
            Err(error) => tracing::warn!(
                listing_id = %listings[representative].id,
                error = %format_args!("{error:#}"),
                "cluster representative failed; using local tailoring for cluster"
            ),
        }

        for member_index in cluster.member_indices {
            if member_index == representative && representative_succeeded {
                continue;
            }
            let listing = &listings[member_index];
            let local = careerai_tailor::tailor_for_listing_local(
                &pool,
                &listing.id,
                &profile,
                cfg.llm.drop_threshold,
            )
            .await
            .with_context(|| format!("local tailor clustered listing {}", listing.id))?;
            outcomes.push(TailoredOutcome {
                application_id: local.application_id,
                listing_title: listing.title.clone(),
                company: listing.company.clone(),
            });
        }
    }
    Ok(outcomes)
}
