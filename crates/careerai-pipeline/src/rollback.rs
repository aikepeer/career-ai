//! Pipeline rollback — move listings and applications backward through the state machine.

use std::path::Path;

use anyhow::{Context, Result};
use careerai_core::state::ListingState;
use careerai_db::queries;
use sqlx::SqlitePool;

use crate::open_pool;

#[derive(Debug, serde::Serialize)]
pub struct RollbackOutcome {
    pub id: String,
    pub from_state: String,
    pub to_state: String,
}

/// Rollback a listing / application to its previous logical state:
/// - `submitted` / `prepared` / `rendered` -> `tailored`
/// - `tailored` -> `shortlisted`
/// - `shortlisted` -> `discovered`
pub async fn rollback_one(root: &Path, id: &str, target: Option<&str>) -> Result<RollbackOutcome> {
    let pool = open_pool(root).await?;
    rollback_one_with_pool(&pool, id, target).await
}

/// Rollback a listing using an already-open pool (avoids opening N+1 pools
/// when called from `rollback_all`).
pub async fn rollback_one_with_pool(
    pool: &SqlitePool,
    id: &str,
    target: Option<&str>,
) -> Result<RollbackOutcome> {
    let listing = if let Ok(l) = queries::find_by_id(pool, id).await {
        l
    } else {
        let app = queries::find_application_by_id(pool, id)
            .await
            .context("find application or listing for rollback")?;
        queries::find_by_id(pool, &app.listing_id)
            .await
            .context("find listing for application")?
    };

    let from_state = listing.state.clone();
    let to_state = match target {
        Some(t) => t.to_string(),
        None => match from_state.as_str() {
            "submitted" | "prepared" | "rendered" => "tailored".to_string(),
            "tailored" => "shortlisted".to_string(),
            _ => "discovered".to_string(),
        },
    };

    let target_listing_state: ListingState = to_state
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid target state '{to_state}': {e}"))?;

    if let Ok(Some(app)) = queries::find_latest_application_for_listing(pool, &listing.id).await {
        if to_state == "shortlisted" || to_state == "discovered" {
            // Wrap deletes + transition in a single transaction so a failed
            // DELETE cannot leave orphaned rows while the state advances.
            let mut tx = pool.begin().await.context("begin rollback transaction")?;
            sqlx::query("DELETE FROM artifacts WHERE application_id = ?")
                .bind(&app.id)
                .execute(&mut *tx)
                .await
                .context("delete artifacts on rollback")?;
            sqlx::query("DELETE FROM application_payloads WHERE application_id = ?")
                .bind(&app.id)
                .execute(&mut *tx)
                .await
                .context("delete application_payloads on rollback")?;
            sqlx::query("DELETE FROM applications WHERE id = ?")
                .bind(&app.id)
                .execute(&mut *tx)
                .await
                .context("delete application on rollback")?;

            let now = chrono::Utc::now();
            sqlx::query("UPDATE listings SET state = ?, updated_at = ? WHERE id = ?")
                .bind(target_listing_state.as_str())
                .bind(now)
                .bind(&listing.id)
                .execute(&mut *tx)
                .await
                .context("update listing state on rollback")?;
            sqlx::query(
                "INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)",
            )
            .bind(&listing.id)
            .bind(&from_state)
            .bind(target_listing_state.as_str())
            .bind("rollback")
            .execute(&mut *tx)
            .await
            .context("insert rollback event")?;
            tx.commit().await.context("commit rollback transaction")?;
        } else {
            queries::transition_application_and_listing(
                pool,
                &app.id,
                &listing.id,
                &to_state,
                target_listing_state,
                Some("rollback"),
            )
            .await
            .context("rollback application and listing")?;
        }
    } else {
        queries::transition(pool, &listing.id, target_listing_state, Some("rollback"))
            .await
            .context("transition listing on rollback")?;
    }

    Ok(RollbackOutcome {
        id: listing.id,
        from_state,
        to_state,
    })
}

/// Rollback all listings matching `from_state` to `to_state`.
pub async fn rollback_all(
    root: &Path,
    from_state: &str,
    to_state: Option<&str>,
) -> Result<Vec<RollbackOutcome>> {
    let pool = open_pool(root).await?;
    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM listings WHERE state = ?")
        .bind(from_state)
        .fetch_all(&pool)
        .await
        .context("fetch listings for rollback")?;

    let mut outcomes = Vec::with_capacity(rows.len());
    for (id,) in rows {
        match rollback_one_with_pool(&pool, &id, to_state).await {
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => {
                tracing::warn!(
                    listing_id = %id,
                    error = %e,
                    "rollback_all: failed to roll back listing"
                );
            }
        }
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_target_state_resolution() {
        let targets = [
            ("rendered", None, "tailored"),
            ("prepared", None, "tailored"),
            ("submitted", None, "tailored"),
            ("tailored", None, "shortlisted"),
            ("shortlisted", None, "discovered"),
            ("rendered", Some("shortlisted"), "shortlisted"),
        ];

        for (from, explicit, expected) in targets {
            let to = match explicit {
                Some(t) => t.to_string(),
                None => match from {
                    "submitted" | "prepared" | "rendered" => "tailored".to_string(),
                    "tailored" => "shortlisted".to_string(),
                    _ => "discovered".to_string(),
                },
            };
            assert_eq!(to, expected);
        }
    }
}
