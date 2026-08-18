#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::error::Result;

use super::is_remote_location;

pub async fn fetch_discovered_explorer(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<crate::view::DiscoveredExplorerItem>> {
    let rows = sqlx::query(
        "SELECT l.id, l.title, l.company, l.location, l.source, l.state, l.score, l.url, \
                l.created_at, \
                (SELECT a.id FROM applications a \
                  WHERE a.listing_id = l.id \
                  ORDER BY a.created_at DESC LIMIT 1) AS application_id \
         FROM listings l ORDER BY l.created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let title: String = row.try_get("title").unwrap_or_default();
        let company: String = row.try_get("company").unwrap_or_default();
        let location: Option<String> = row.try_get("location").ok();
        let source: String = row
            .try_get("source")
            .unwrap_or_else(|_| "unknown".to_string());
        let state: String = row.try_get("state").unwrap_or_default();
        let score_f64: Option<f64> = row.try_get("score").ok();
        let score = score_f64.map(|s| s as f32);
        let is_remote = location.as_deref().is_some_and(is_remote_location);
        let url: String = row.try_get("url").unwrap_or_default();
        let application_id: Option<String> = row.try_get("application_id").ok().flatten();
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();

        out.push(crate::view::DiscoveredExplorerItem {
            id,
            title,
            company,
            location,
            source,
            state,
            score,
            is_remote,
            url,
            application_id,
            created_at,
        });
    }
    Ok(out)
}
