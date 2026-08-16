// Numeric conversions in this module are small-domain (counts in the
// millions at worst, scores in [0.0, 1.0]) so the precision-loss /
// truncation lints are noise here.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use careerai_core::state::ListingState;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::error::Result;
use crate::view::{FunnelColumn, KpiStrip, ListingCard, PipelineSnapshot, StateCounts};

const FUNNEL_STATES: &[ListingState] = &[
    ListingState::Discovered,
    ListingState::Shortlisted,
    ListingState::Tailored,
    ListingState::Rendered,
    ListingState::Submitted,
    ListingState::Responded,
];

pub async fn snapshot(pool: &SqlitePool) -> Result<PipelineSnapshot> {
    let kpi = kpi_strip(pool).await?;
    let columns = funnel_columns(pool).await?;
    let state_counts = StateCounts {
        shortlisted: count_state(pool, ListingState::Shortlisted).await?,
        tailored: count_state(pool, ListingState::Tailored).await?,
        rendered: count_state(pool, ListingState::Rendered).await?,
    };
    let source_lag_hours = source_lag(pool).await?;
    Ok(PipelineSnapshot {
        kpi,
        columns,
        state_counts,
        source_lag_hours,
        linkedin_cookie_days_left: None,
        profile_age_days: None,
    })
}

async fn kpi_strip(pool: &SqlitePool) -> Result<KpiStrip> {
    let today_discovered = count_today_any_state(pool).await?;
    let shortlisted_active = count_state(pool, ListingState::Shortlisted).await?;
    let submitted = count_state(pool, ListingState::Submitted).await?;
    let responded = count_state(pool, ListingState::Responded).await?;
    let applied_lifetime = submitted + responded;
    let response_rate_pct = if applied_lifetime == 0 {
        None
    } else {
        let r = responded as f64 / applied_lifetime as f64 * 100.0;
        Some(r as f32)
    };
    let response_rate_label =
        response_rate_pct.map_or_else(|| "—".to_string(), |p| format!("{}%", p.floor() as i64));
    Ok(KpiStrip {
        today_discovered,
        shortlisted_active,
        applied_lifetime,
        response_rate_pct,
        response_rate_label,
    })
}

async fn funnel_columns(pool: &SqlitePool) -> Result<Vec<FunnelColumn>> {
    let mut out = Vec::with_capacity(FUNNEL_STATES.len());
    for state in FUNNEL_STATES {
        let count = count_state(pool, *state).await?;
        let top = top_listings(pool, *state).await?;
        out.push(FunnelColumn {
            state: *state,
            label: label_for(*state),
            count,
            top,
        });
    }
    Ok(out)
}

fn label_for(state: ListingState) -> String {
    match state {
        ListingState::Discovered => "Discovered",
        ListingState::FilteredOut => "Filtered Out",
        ListingState::Shortlisted => "Shortlisted",
        ListingState::Tailored => "Tailored",
        ListingState::Rendered => "Rendered",
        ListingState::Prepared => "Prepared",
        ListingState::Drafted => "Drafted",
        ListingState::Submitted => "Applied",
        ListingState::Responded => "Responded",
        ListingState::Skipped => "Skipped",
        ListingState::Failed => "Failed",
    }
    .to_string()
}

async fn count_state(pool: &SqlitePool, state: ListingState) -> Result<u64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM listings WHERE state = ?")
        .bind(state.as_str())
        .fetch_one(pool)
        .await
        .map_err(careerai_db::error::DbError::from)?;
    let n: i64 = row
        .try_get("n")
        .map_err(careerai_db::error::DbError::from)?;
    Ok(u64::try_from(n.max(0)).unwrap_or(0))
}

async fn count_today_any_state(pool: &SqlitePool) -> Result<u64> {
    let start_of_day = today_start();
    let row = sqlx::query("SELECT COUNT(*) AS n FROM listings WHERE created_at >= ?")
        .bind(start_of_day)
        .fetch_one(pool)
        .await
        .map_err(careerai_db::error::DbError::from)?;
    let n: i64 = row
        .try_get("n")
        .map_err(careerai_db::error::DbError::from)?;
    Ok(u64::try_from(n.max(0)).unwrap_or(0))
}

async fn top_listings(pool: &SqlitePool, state: ListingState) -> Result<Vec<ListingCard>> {
    let rows = sqlx::query(
        "SELECT id, title, company, score, url, created_at \
         FROM listings WHERE state = ? \
         ORDER BY score DESC NULLS LAST, created_at DESC",
    )
    .bind(state.as_str())
    .fetch_all(pool)
    .await
    .map_err(careerai_db::error::DbError::from)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row
            .try_get("id")
            .map_err(careerai_db::error::DbError::from)?;
        let title: String = row
            .try_get("title")
            .map_err(careerai_db::error::DbError::from)?;
        let company: String = row
            .try_get("company")
            .map_err(careerai_db::error::DbError::from)?;
        let score_f64: Option<f64> = row.try_get("score").ok();
        let score = score_f64.map(|s| s as f32);
        let url: String = row
            .try_get("url")
            .map_err(careerai_db::error::DbError::from)?;
        let posted_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();
        out.push(ListingCard {
            id,
            title,
            company,
            score,
            url,
            posted_at,
        });
    }
    Ok(out)
}

async fn source_lag(pool: &SqlitePool) -> Result<Vec<(String, u64)>> {
    let rows =
        sqlx::query("SELECT source, MAX(created_at) AS last_at FROM listings GROUP BY source")
            .fetch_all(pool)
            .await
            .map_err(careerai_db::error::DbError::from)?;
    let now = Utc::now();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let source: String = row
            .try_get("source")
            .map_err(careerai_db::error::DbError::from)?;
        let last_at: Option<DateTime<Utc>> = row.try_get("last_at").ok();
        if let Some(ts) = last_at {
            let delta = now.signed_duration_since(ts);
            let hours = u64::try_from(delta.num_hours().max(0)).unwrap_or(0);
            out.push((source, hours));
        }
    }
    Ok(out)
}

fn today_start() -> DateTime<Utc> {
    let now = Utc::now();
    match now.date_naive().and_hms_opt(0, 0, 0) {
        Some(naive) => DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc),
        None => now,
    }
}
