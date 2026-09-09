//! Read-only activity for the workspace. Dry runs never create submitted events.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Serialize)]
pub struct ActivityDay {
    pub date: String,
    pub submitted: i64,
    pub responded: i64,
}

#[derive(Debug, Serialize)]
pub struct ActivitySummary {
    pub week_start: String,
    pub submitted_this_week: i64,
    pub days: Vec<ActivityDay>,
}

/// Count distinct listings per day and per calendar week, bounded by now.
/// UTC is explicit in the UI; duplicate audit events cannot inflate the goal.
pub async fn summary_at(pool: &SqlitePool, now: DateTime<Utc>) -> crate::Result<ActivitySummary> {
    let today = now.date_naive();
    let week_start = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let start = today - Duration::days(6);
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT date(created_at),
                COUNT(DISTINCT CASE WHEN to_state = 'submitted' THEN listing_id END),
                COUNT(DISTINCT CASE WHEN to_state = 'responded' THEN listing_id END)
         FROM events
         WHERE julianday(created_at) >= julianday(?) AND julianday(created_at) <= julianday(?)
           AND to_state IN ('submitted', 'responded')
         GROUP BY date(created_at)",
    )
    .bind(start.to_string())
    .bind(now.to_rfc3339())
    .fetch_all(pool)
    .await
    .map_err(careerai_db::DbError::from)?;
    let (submitted_this_week,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT listing_id) FROM events
         WHERE to_state = 'submitted'
           AND julianday(created_at) >= julianday(?) AND julianday(created_at) <= julianday(?)",
    )
    .bind(week_start.to_string())
    .bind(now.to_rfc3339())
    .fetch_one(pool)
    .await
    .map_err(careerai_db::DbError::from)?;
    let days = (0..7)
        .map(|offset| {
            let date = (start + Duration::days(offset)).to_string();
            let counts = rows.iter().find(|row| row.0 == date);
            ActivityDay {
                date,
                submitted: counts.map_or(0, |row| row.1),
                responded: counts.map_or(0, |row| row.2),
            }
        })
        .collect();
    Ok(ActivitySummary {
        week_start: week_start.to_string(),
        submitted_this_week,
        days,
    })
}

pub async fn api_activity(State(state): State<Arc<crate::AppState>>) -> impl IntoResponse {
    match summary_at(&state.pool, Utc::now()).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(error) => {
            tracing::error!(%error, "workspace activity query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Activity could not be loaded. Please retry." })),
            )
                .into_response()
        }
    }
}
