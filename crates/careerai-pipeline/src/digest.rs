//! `digest_summary` — daily / N-hour rollup of pipeline activity
//! by state and by source. Powers `careerai digest --since <window>`.

use std::path::Path;

use anyhow::{Context, Result};

use crate::{open_pool, DigestReport, SourceCounts};

pub async fn digest_summary(root: &Path, since: chrono::Duration) -> Result<DigestReport> {
    use sqlx::Row;

    let pool = open_pool(root).await?;
    let cutoff = chrono::Utc::now() - since;
    // Format with millisecond precision to match SQLite's
    // `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` (the `%f` modifier emits
    // `SS.SSS` — three digits, milliseconds). The WHERE clause compares
    // events.created_at >= cutoff_iso as TEXT, so the two formats MUST
    // produce byte-equivalent strings at equal instants — otherwise an
    // event at the exact cutoff could sort as either before or after,
    // causing off-by-microsecond inclusion bugs at the boundary.
    let cutoff_iso = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let mut report = DigestReport {
        since_iso: cutoff_iso.clone(),
        ..DigestReport::default()
    };

    // Per-state counts: distinct listing_id grouped by to_state in window.
    let state_rows = sqlx::query(
        "SELECT to_state, COUNT(DISTINCT listing_id) AS n
         FROM events
         WHERE created_at >= ?
         GROUP BY to_state",
    )
    .bind(&cutoff_iso)
    .fetch_all(&pool)
    .await
    .context("digest: per-state counts")?;

    for row in state_rows {
        let to_state: String = row.try_get("to_state")?;
        let n: i64 = row.try_get("n")?;
        // COUNT(DISTINCT ...) is non-negative by SQL spec; a negative
        // here means schema corruption. Surface loudly rather than
        // silently zeroing — a wrong count masquerading as 0 is the
        // worst possible outcome for a reporting function.
        let n: usize = n.try_into().with_context(|| {
            format!("digest: COUNT returned negative ({n}) for state {to_state}")
        })?;
        match to_state.as_str() {
            "discovered" => report.discovered = n,
            "filtered_out" => report.matched += n,
            "shortlisted" => {
                report.shortlisted = n;
                report.matched += n;
            }
            "drafted" => report.drafted = n,
            "submitted" => report.submitted = n,
            "failed" => report.failed = n,
            "responded" => report.responded = n,
            // Tailored / Rendered / Prepared / Skipped not surfaced in
            // the digest summary — they're transient pipeline stages
            // rather than operator-meaningful outcomes.
            _ => {}
        }
    }

    // Per-source: distinct listings with any transition in window.
    // LOWER(l.source) merges "linkedin" / "LinkedIn" / "LINKEDIN" into
    // a single bucket and lower-cases the result key (so the CLI doesn't
    // have to). LOWER() is the function form vs PR #14's `COLLATE
    // NOCASE` predicate trick — both produce the same merge, but here
    // we're projecting (SELECT) rather than filtering (WHERE), so
    // function form is the natural choice. listings.source isn't
    // lowercase-enforced by the schema, and submit_application's
    // dispatch already does to_ascii_lowercase, so we follow suit.
    let source_rows = sqlx::query(
        "SELECT LOWER(l.source) AS source, COUNT(DISTINCT e.listing_id) AS n
         FROM events e
         JOIN listings l ON l.id = e.listing_id
         WHERE e.created_at >= ?
         GROUP BY LOWER(l.source)",
    )
    .bind(&cutoff_iso)
    .fetch_all(&pool)
    .await
    .context("digest: per-source counts")?;

    for row in source_rows {
        let source: String = row.try_get("source")?;
        let n: i64 = row.try_get("n")?;
        let total: usize = n.try_into().with_context(|| {
            format!("digest: COUNT returned negative ({n}) for source {source}")
        })?;
        report.per_source.insert(source, SourceCounts { total });
    }

    // last_tick: most recent event in the database (not bounded by
    // window). MAX(created_at) on an empty `events` returns one row
    // containing NULL. `fetch_optional` only makes the *row* optional,
    // not the column value, so decode the column as `Option<String>`.
    let last: Option<(Option<String>,)> = sqlx::query_as("SELECT MAX(created_at) FROM events")
        .fetch_optional(&pool)
        .await
        .context("digest: last_tick")?;
    report.last_tick = last.and_then(|(s,)| s).filter(|s| !s.is_empty());

    Ok(report)
}
