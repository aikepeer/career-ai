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
    // R08: reject forward target states. Rollback must only move
    // backward through the pipeline — a target of `submitted`,
    // `prepared`, `rendered`, or `tailored` from a `shortlisted` or
    // `discovered` listing would advance the listing, not roll it back.
    let to_state = match target {
        Some(t) => {
            let parsed: ListingState = t
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid target state '{t}': {e}"))?;
            let from_parsed: ListingState = from_state
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid current state '{from_state}': {e}"))?;
            if !is_backward_transition(from_parsed, parsed) {
                anyhow::bail!("rollback target '{t}' is not backward from state '{from_state}'");
            }
            t.to_string()
        }
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
            rollback_with_application_delete(
                pool,
                &app.id,
                &listing.id,
                &from_state,
                target_listing_state,
            )
            .await?;
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
/// R08: delete dependent rows and the application, then transition the
/// listing, all in one transaction. Dependent rows are deleted before
/// the application to satisfy FK constraints (follow_ups, variants,
/// feedback lack ON DELETE CASCADE).
async fn rollback_with_application_delete(
    pool: &SqlitePool,
    app_id: &str,
    listing_id: &str,
    from_state: &str,
    target_listing_state: ListingState,
) -> Result<()> {
    let mut tx = pool.begin().await.context("begin rollback transaction")?;

    sqlx::query("DELETE FROM follow_ups WHERE application_id = ?")
        .bind(app_id)
        .execute(&mut *tx)
        .await
        .context("delete follow_ups on rollback")?;
    sqlx::query("DELETE FROM application_variants WHERE application_id = ?")
        .bind(app_id)
        .execute(&mut *tx)
        .await
        .context("delete application_variants on rollback")?;
    sqlx::query("DELETE FROM interview_feedback WHERE application_id = ?")
        .bind(app_id)
        .execute(&mut *tx)
        .await
        .context("delete interview_feedback on rollback")?;
    sqlx::query("DELETE FROM artifacts WHERE application_id = ?")
        .bind(app_id)
        .execute(&mut *tx)
        .await
        .context("delete artifacts on rollback")?;
    sqlx::query("DELETE FROM application_payloads WHERE application_id = ?")
        .bind(app_id)
        .execute(&mut *tx)
        .await
        .context("delete application_payloads on rollback")?;
    sqlx::query("DELETE FROM applications WHERE id = ?")
        .bind(app_id)
        .execute(&mut *tx)
        .await
        .context("delete application on rollback")?;

    let now = chrono::Utc::now();
    sqlx::query("UPDATE listings SET state = ?, updated_at = ? WHERE id = ?")
        .bind(target_listing_state.as_str())
        .bind(now)
        .bind(listing_id)
        .execute(&mut *tx)
        .await
        .context("update listing state on rollback")?;
    sqlx::query("INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)")
        .bind(listing_id)
        .bind(from_state)
        .bind(target_listing_state.as_str())
        .bind("rollback")
        .execute(&mut *tx)
        .await
        .context("insert rollback event")?;
    tx.commit().await.context("commit rollback transaction")?;
    Ok(())
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
/// R08: returns true if `to` is a backward (or same-level) transition
/// from `from`. The pipeline order is:
///   discovered → shortlisted → tailored → rendered → prepared → submitted → responded
/// Backward means `to` appears at or before `from` in this sequence.
/// `failed` and `skipped` are terminal side-states; rolling back from
/// them to any main-line state is allowed.
fn is_backward_transition(from: ListingState, to: ListingState) -> bool {
    let rank = |s: ListingState| -> usize {
        match s {
            ListingState::Discovered | ListingState::FilteredOut => 0,
            ListingState::Shortlisted => 1,
            ListingState::Tailored => 2,
            ListingState::Rendered | ListingState::Drafted => 3,
            ListingState::Prepared => 4,
            ListingState::Submitted => 5,
            ListingState::Responded => 6,
            ListingState::Failed | ListingState::Skipped => 7,
        }
    };
    // If from is a terminal state, any non-terminal target is backward.
    let from_rank = rank(from);
    let to_rank = rank(to);
    if from_rank == 7 {
        return to_rank < 7;
    }
    to_rank <= from_rank
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
