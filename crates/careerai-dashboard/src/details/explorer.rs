#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::error::Result;

use super::is_remote_location;

pub async fn fetch_discovered_explorer(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<crate::view::DiscoveredExplorerItem>> {
    // R17: delegate to the filtered query with no filters and the given
    // limit, preserving backward compatibility for existing callers.
    fetch_explorer_filtered(pool, &ExplorerFilter::default_with_limit(limit)).await
}

/// R17: server-side filter parameters for the explorer. All optional —
/// omitted filters match everything. `limit` caps the returned rows;
/// `offset` enables pagination.
#[derive(Debug, Clone, Default)]
pub struct ExplorerFilter {
    pub query: Option<String>,
    pub source: Option<String>,
    pub state: Option<String>,
    pub remote_only: bool,
    pub limit: u32,
    pub offset: u32,
}

impl ExplorerFilter {
    /// Backward-compatible default: no filters, just a limit.
    pub fn default_with_limit(limit: u32) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }
}

/// R17: filtered + paginated explorer query. Builds a WHERE clause from
/// the filter parameters so the search covers every stored listing, not
/// just the first 10,000. The `query` filter searches title + company
/// (case-insensitive LIKE). `remote_only` filters on the location column
/// containing common remote indicators.
pub async fn fetch_explorer_filtered(
    pool: &SqlitePool,
    filter: &ExplorerFilter,
) -> Result<Vec<crate::view::DiscoveredExplorerItem>> {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref q) = filter.query {
        let pat = format!("%{q}%");
        where_clauses.push(
            "(LOWER(l.title) LIKE LOWER(?) OR LOWER(l.company) LIKE LOWER(?))".to_string(),
        );
        binds.push(pat.clone());
        binds.push(pat);
    }
    if let Some(ref source) = filter.source {
        where_clauses.push("l.source = ?".to_string());
        binds.push(source.clone());
    }
    if let Some(ref state) = filter.state {
        where_clauses.push("l.state = ?".to_string());
        binds.push(state.clone());
    }
    if filter.remote_only {
        where_clauses.push(
            "LOWER(l.location) LIKE '%remote%' OR l.location IS NULL".to_string(),
        );
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let sql = format!(
        "SELECT l.id, l.title, l.company, l.location, l.source, l.state, l.score, l.url, \
                l.description, l.created_at, \
                (SELECT a.id FROM applications a \
                  WHERE a.listing_id = l.id \
                  ORDER BY a.created_at DESC LIMIT 1) AS application_id \
         FROM listings l {where_sql} \
         ORDER BY l.created_at DESC LIMIT ? OFFSET ?"
    );

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(filter.limit);
    q = q.bind(filter.offset);

    let rows = q
        .fetch_all(pool)
        .await
        .map_err(careerai_db::DbError::from)?;

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
        let description: String = row.try_get("description").unwrap_or_default();

        // Legitimacy + eligibility badges (cheap heuristics, no LLM).
        let raw = careerai_sources::base::RawListing {
            source: source.clone(),
            external_id: id.clone(),
            title: title.clone(),
            company: company.clone(),
            location: location.clone(),
            url: url.clone(),
            description,
            raw_json: None,
        };
        let legitimacy = match careerai_match::assess_legitimacy(&raw) {
            careerai_match::LegitimacyScore::HighConfidence => "high_confidence",
            careerai_match::LegitimacyScore::ProceedWithCaution => "caution",
            careerai_match::LegitimacyScore::Suspicious => "suspicious",
        };
        let eligibility = "unknown"; // Work authorization has not been evaluated here.

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
            legitimacy: legitimacy.to_string(),
            eligibility: eligibility.to_string(),
        });
    }
    Ok(out)
}

/// R17: count of listings matching the given filter, for pagination total.
pub async fn fetch_explorer_filtered_count(
    pool: &SqlitePool,
    filter: &ExplorerFilter,
) -> Result<u64> {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref q) = filter.query {
        let pat = format!("%{q}%");
        where_clauses.push(
            "(LOWER(l.title) LIKE LOWER(?) OR LOWER(l.company) LIKE LOWER(?))".to_string(),
        );
        binds.push(pat.clone());
        binds.push(pat);
    }
    if let Some(ref source) = filter.source {
        where_clauses.push("l.source = ?".to_string());
        binds.push(source.clone());
    }
    if let Some(ref state) = filter.state {
        where_clauses.push("l.state = ?".to_string());
        binds.push(state.clone());
    }
    if filter.remote_only {
        where_clauses.push(
            "LOWER(l.location) LIKE '%remote%' OR l.location IS NULL".to_string(),
        );
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let sql = format!("SELECT COUNT(*) AS n FROM listings l {where_sql}");

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }

    let row = q
        .fetch_one(pool)
        .await
        .map_err(careerai_db::DbError::from)?;
    let n: i64 = Row::try_get(&row, "n").unwrap_or(0);
    #[allow(clippy::cast_sign_loss)]
    {
        Ok(n.max(0) as u64)
    }
}
/// Used by `render_index` so the page renders immediately while rows
/// load lazily via `/api/v1/explorer`.
pub async fn fetch_explorer_count(pool: &SqlitePool) -> Result<u64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM listings")
        .fetch_one(pool)
        .await
        .map_err(careerai_db::DbError::from)?;
    let n: i64 = Row::try_get(&row, "n").unwrap_or(0);
    #[allow(clippy::cast_sign_loss)]
    {
        Ok(n.max(0) as u64)
    }
}
