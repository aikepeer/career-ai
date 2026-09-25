#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::error::Result;
use crate::view::ActionItem;

pub async fn fetch_action_center(pool: &SqlitePool) -> Result<Vec<ActionItem>> {
    let rows = sqlx::query(
        "SELECT id, source, external_id, title, company, state, score, url, created_at \
         FROM listings WHERE state IN ('drafted', 'responded') ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(careerai_db::error::DbError::from)?;

    let mut items = Vec::new();
    for row in rows {
        let listing_id: String = row
            .try_get("id")
            .map_err(careerai_db::error::DbError::from)?;
        let title: String = row
            .try_get("title")
            .map_err(careerai_db::error::DbError::from)?;
        let company: String = row
            .try_get("company")
            .map_err(careerai_db::error::DbError::from)?;
        let state: String = row
            .try_get("state")
            .map_err(careerai_db::error::DbError::from)?;
        let score_f64: Option<f64> = row.try_get("score").ok();
        let score = score_f64.map(|s| s as f32);
        let url: String = row
            .try_get("url")
            .map_err(careerai_db::error::DbError::from)?;
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();

        let (action_type, action_label, action_url) = if state == "drafted" {
            (
                "review".to_string(),
                "Review Resume & Letter Draft".to_string(),
                None,
            )
        } else {
            (
                "prep".to_string(),
                "Open Interview Study Sheet".to_string(),
                Some(url.clone()),
            )
        };

        items.push(ActionItem {
            id: listing_id.clone(),
            listing_id,
            title,
            company,
            state,
            score,
            action_type,
            action_label,
            action_url,
            created_at,
        });
    }
    Ok(items)
}
