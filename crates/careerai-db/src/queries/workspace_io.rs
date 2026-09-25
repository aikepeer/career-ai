//! F10: Portable workspace export and restore.
//!
//! Exports the SQLite database (listings, applications, payloads, events,
//! outcomes, follow-ups, match reasons) to a single JSON file. Restore
//! loads the JSON back into a fresh database, preserving relationships.
//!
//! Secrets (API keys, cookies, SMTP creds) are NOT stored in the DB and
//! are therefore never exported. The export is safe to share.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::Result;

/// Top-level export container. Versioned for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceExport {
    /// Export format version — bump on breaking schema changes.
    pub format_version: u32,
    /// ISO 8601 timestamp when the export was created.
    pub exported_at: String,
    /// Table snapshots, keyed by table name.
    pub tables: ExportTables,
}

/// Per-table row collections. Each table is a `Vec<serde_json::Value>`
/// so the export is resilient to schema additions (extra columns are
/// preserved, missing columns deserialize to `null`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExportTables {
    pub listings: Vec<serde_json::Value>,
    pub applications: Vec<serde_json::Value>,
    pub application_payloads: Vec<serde_json::Value>,
    pub events: Vec<serde_json::Value>,
    pub outcomes: Vec<serde_json::Value>,
    pub follow_ups: Vec<serde_json::Value>,
    pub match_reasons: Vec<serde_json::Value>,
}

/// Export all workspace tables to a `WorkspaceExport`.
///
/// # Errors
/// Returns a `DbError` if any table query fails.
pub async fn export_workspace(pool: &SqlitePool) -> Result<WorkspaceExport> {
    let listings = export_table_json(pool, "SELECT * FROM listings").await?;
    let applications = export_table_json(pool, "SELECT * FROM applications").await?;
    let payloads = export_table_json(pool, "SELECT * FROM application_payloads").await?;
    let events = export_table_json(pool, "SELECT * FROM events").await?;
    let outcomes = export_table_json(pool, "SELECT * FROM employer_outcomes").await?;
    let follow_ups = export_table_json(pool, "SELECT * FROM follow_ups").await?;
    let reasons = export_table_json(pool, "SELECT * FROM match_reasons").await?;

    Ok(WorkspaceExport {
        format_version: 1,
        exported_at: chrono::Utc::now().to_rfc3339(),
        tables: ExportTables {
            listings,
            applications,
            application_payloads: payloads,
            events,
            outcomes,
            follow_ups,
            match_reasons: reasons,
        },
    })
}

/// Restore a `WorkspaceExport` into the database. Inserts rows using
/// `INSERT OR IGNORE` so restoring into an existing workspace does not
/// overwrite or duplicate existing rows.
///
/// # Errors
/// Returns a `DbError` if any insert fails.
pub async fn restore_workspace(pool: &SqlitePool, export: &WorkspaceExport) -> Result<()> {
    restore_table(pool, "listings", &export.tables.listings).await?;
    restore_table(pool, "applications", &export.tables.applications).await?;
    restore_table(
        pool,
        "application_payloads",
        &export.tables.application_payloads,
    )
    .await?;
    restore_table(pool, "events", &export.tables.events).await?;
    restore_table(pool, "employer_outcomes", &export.tables.outcomes).await?;
    restore_table(pool, "follow_ups", &export.tables.follow_ups).await?;
    restore_table(pool, "match_reasons", &export.tables.match_reasons).await?;
    Ok(())
}

async fn restore_table(pool: &SqlitePool, table: &str, rows: &[serde_json::Value]) -> Result<()> {
    for row in rows {
        let Some(obj) = row.as_object() else {
            continue;
        };
        let columns: Vec<&str> = obj.keys().map(std::string::String::as_str).collect();
        let col_list = columns.join(", ");
        let placeholders: Vec<&str> = columns.iter().map(|_| "?").collect();
        let sql = format!(
            "INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({ph})",
            ph = placeholders.join(", ")
        );
        let mut query = sqlx::query(&sql);
        for col in &columns {
            let val = &obj[*col];
            query = match val {
                serde_json::Value::Null => query.bind(None::<String>),
                serde_json::Value::Bool(b) => query.bind(b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        query.bind(i)
                    } else if let Some(f) = n.as_f64() {
                        query.bind(f)
                    } else {
                        query.bind(n.to_string())
                    }
                }
                serde_json::Value::String(s) => query.bind(s.clone()),
                _ => query.bind(val.to_string()),
            };
        }
        query.execute(pool).await?;
    }
    Ok(())
}
/// Query a table and convert each row to a `serde_json::Value` object,
/// keyed by column name. This is schema-agnostic: new columns are
/// picked up automatically without changing the export code.
async fn export_table_json(pool: &SqlitePool, query: &str) -> Result<Vec<serde_json::Value>> {
    use sqlx::{Column, Row};
    let rows = sqlx::query(query).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut obj = serde_json::Map::new();
        for col in row.columns() {
            let col_name = col.name();
            let val = row_value_to_json(&row, col_name);
            obj.insert(col_name.to_string(), val);
        }
        out.push(serde_json::Value::Object(obj));
    }
    Ok(out)
}

fn row_value_to_json(row: &sqlx::sqlite::SqliteRow, col_name: &str) -> serde_json::Value {
    use sqlx::Row;
    // SQLite is dynamically typed — try the most common types in order.
    // `try_get` returns Err for type mismatches, which we map to Null.
    if let Ok(v) = row.try_get::<Option<i64>, _>(col_name) {
        return v.map_or(serde_json::Value::Null, serde_json::Value::from);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(col_name) {
        return v.map_or(serde_json::Value::Null, serde_json::Value::from);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(col_name) {
        return v.map_or(serde_json::Value::Null, serde_json::Value::from);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(col_name) {
        return v.map_or(serde_json::Value::Null, serde_json::Value::from);
    }
    serde_json::Value::Null
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;

    #[tokio::test]
    async fn export_round_trips_through_restore() {
        let pool = pool_in_memory().await.unwrap();

        // The in-memory pool starts with migrations applied. Export
        // the empty workspace, then restore it into a fresh pool.
        let export = export_workspace(&pool).await.unwrap();
        assert_eq!(export.format_version, 1);
        assert!(export.tables.listings.is_empty());

        let pool2 = pool_in_memory().await.unwrap();
        restore_workspace(&pool2, &export).await.unwrap();

        // Restoring into a fresh pool should yield an identical export.
        let export2 = export_workspace(&pool2).await.unwrap();
        assert_eq!(export2.tables.listings.len(), export.tables.listings.len());
    }

    #[tokio::test]
    async fn export_captures_seeded_listings() {
        use crate::models::NewListing;
        use crate::queries;

        let pool = pool_in_memory().await.unwrap();
        queries::insert_or_ignore(
            &pool,
            &NewListing {
                source: "greenhouse".into(),
                external_id: "exp-1".into(),
                title: "Rust Dev".into(),
                company: "Acme".into(),
                location: Some("Remote".into()),
                url: "https://example.com/1".into(),
                description: "Build in Rust.".into(),
                raw_json: None,
            },
        )
        .await
        .unwrap();

        let export = export_workspace(&pool).await.unwrap();
        assert_eq!(export.tables.listings.len(), 1);

        // Restore into a fresh pool and verify the listing round-trips.
        let pool2 = pool_in_memory().await.unwrap();
        restore_workspace(&pool2, &export).await.unwrap();
        let export2 = export_workspace(&pool2).await.unwrap();
        assert_eq!(export2.tables.listings.len(), 1);
    }
}
