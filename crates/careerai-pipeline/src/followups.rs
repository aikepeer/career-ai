//! Follow-up report (ported from career-ops `followup-cadence.mjs`).

use std::path::Path;

use anyhow::{Context, Result};

use careerai_db::queries;

use crate::open_pool;

/// One submitted application that has gone quiet.
#[derive(Debug, Clone)]
pub struct FollowupItem {
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub source: String,
    pub url: String,
    pub days_since: i64,
}

/// List submitted applications with no response for at least `min_days`.
pub async fn list_followups(root: &Path, min_days: i64) -> Result<Vec<FollowupItem>> {
    let pool = open_pool(root).await?;
    let rows = queries::list_stale_submissions(&pool, min_days)
        .await
        .context("list stale submissions")?;
    let now = chrono::Utc::now();
    Ok(rows
        .into_iter()
        .map(|r| {
            let days_since = (now - r.submitted_at).num_days().max(0);
            FollowupItem {
                listing_id: r.listing_id,
                title: r.title,
                company: r.company,
                source: r.source,
                url: r.url,
                days_since,
            }
        })
        .collect())
}
