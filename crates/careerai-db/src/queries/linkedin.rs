//! LinkedIn-specific draft-review queries. The `careerai review`
//! flow parks LinkedIn submissions in `state = drafted` (because
//! linkedin_browser's `interactive_only = true` config prevents
//! auto-submit) so the operator can confirm each one. These queries
//! are scoped to that flow.

use sqlx::SqlitePool;

use crate::error::Result;
use crate::models::Application;

/// List applications in state `drafted` whose listing source is `linkedin`,
/// oldest first (so operators review in submission order).
///
/// Used by `careerai review` to enumerate the queue of applications that the
/// daemon has parked in `Drafted` due to `interactive_only = true`.
///
/// `limit` is clamped to `[1, 1_000]` before binding. Without this, a
/// negative value would disable the LIMIT in SQLite and a zero-or-negative
/// would silently return nothing — both surprising for a "bounded" query.
///
/// `l.source = 'linkedin' COLLATE NOCASE` mirrors the case-insensitive
/// lookup in `careerai-submit::submit_application`. The schema doesn't
/// enforce lowercase on `listings.source`, so a row inserted as
/// `"LinkedIn"` would otherwise be invisible to `careerai review`.
/// COLLATE NOCASE is sargable — unlike `LOWER(l.source)` which would
/// force a function evaluation per row and prevent SQLite from using
/// the `idx_listings_source_state` index on `listings(source, state)`.
pub async fn list_drafted_linkedin(pool: &SqlitePool, limit: i64) -> Result<Vec<Application>> {
    const MIN_LIMIT: i64 = 1;
    const MAX_LIMIT: i64 = 1_000;
    let limit = limit.clamp(MIN_LIMIT, MAX_LIMIT);

    let rows = sqlx::query_as::<_, Application>(
        "SELECT a.id, a.listing_id, a.state, a.profile_hash, a.prompt_version, a.llm_model,
                a.created_at, a.updated_at
         FROM applications a
         JOIN listings l ON l.id = a.listing_id
         WHERE a.state = 'drafted' AND l.source = 'linkedin' COLLATE NOCASE
         ORDER BY a.created_at ASC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Atomically "claim" a Drafted application for submission by transitioning
/// it to `Rendered`. Returns `true` iff the claim won.
///
/// This is the race guard for `careerai review`: two operators (or two
/// review processes) can both call `confirm_linkedin_submit` on the same
/// row, see `state = 'drafted'`, and try to submit. SQLite serializes
/// writes, so the conditional UPDATE matches the row exactly once; the
/// loser sees `rows_affected == 0` and bails before launching a browser.
///
/// `Rendered` is reused (rather than introducing a new `Submitting` state)
/// because `submit_application`'s state guard already accepts it and the
/// downstream success/failure transitions remain coherent. After a
/// successful submit the row goes to `Submitted`; on a `submit_application`
/// failure path the row goes to `Failed`.
///
/// **Crash recovery.** If the process crashes after the claim succeeds but
/// before `submit_application` runs (or while it's running, before its
/// failure transition), the row stays in `Rendered`. It is no longer
/// returned by `list_drafted_linkedin`, so the operator won't see it in
/// `careerai review`. Recovery: inspect with `careerai inspect <id>` and
/// either resubmit by setting state back to `drafted` and re-running
/// review, or call `careerai apply <id>` directly (Rendered is a valid
/// input state for the submit layer).
pub async fn claim_drafted_application(pool: &SqlitePool, application_id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE applications
         SET state = 'rendered',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ? AND state = 'drafted'",
    )
    .bind(application_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::applications::{create_application, set_application_state};
    use crate::queries::listings::insert_or_ignore;
    use crate::queries::test_support::{fixture, new_app};

    #[tokio::test]
    async fn list_drafted_linkedin_returns_only_drafted_linkedin() {
        let pool = pool_in_memory().await.unwrap();

        // Listing A + B: linkedin source
        let (li_id_a, _) = insert_or_ignore(&pool, &fixture("linkedin", "li-dl-a"))
            .await
            .unwrap();
        let (li_id_b, _) = insert_or_ignore(&pool, &fixture("linkedin", "li-dl-b"))
            .await
            .unwrap();
        // Listing C: linkedin but application ends up in `rendered` (wrong state)
        let (li_id_c, _) = insert_or_ignore(&pool, &fixture("linkedin", "li-dl-c"))
            .await
            .unwrap();
        // Listing D: greenhouse source (wrong source)
        let (gh_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "gh-dl-d"))
            .await
            .unwrap();

        // A: drafted + linkedin
        let app_a = create_application(&pool, &new_app(&li_id_a)).await.unwrap();
        set_application_state(&pool, &app_a.id, "drafted")
            .await
            .unwrap();

        // B: drafted + linkedin
        let app_b = create_application(&pool, &new_app(&li_id_b)).await.unwrap();
        set_application_state(&pool, &app_b.id, "drafted")
            .await
            .unwrap();

        // C: rendered + linkedin (wrong state — must not appear)
        let app_c = create_application(&pool, &new_app(&li_id_c)).await.unwrap();
        set_application_state(&pool, &app_c.id, "rendered")
            .await
            .unwrap();

        // D: drafted + greenhouse (wrong source — must not appear)
        let app_d = create_application(&pool, &new_app(&gh_id)).await.unwrap();
        set_application_state(&pool, &app_d.id, "drafted")
            .await
            .unwrap();

        let rows = list_drafted_linkedin(&pool, 100).await.unwrap();
        assert_eq!(rows.len(), 2, "must return only drafted+linkedin rows");
        for r in &rows {
            assert_eq!(r.state, "drafted", "unexpected state: {}", r.state);
        }
        // IDs must be A and B (order is oldest-first, which is insertion order here)
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(
            ids.contains(&app_a.id.as_str()),
            "app_a missing from results"
        );
        assert!(
            ids.contains(&app_b.id.as_str()),
            "app_b missing from results"
        );
    }
}
