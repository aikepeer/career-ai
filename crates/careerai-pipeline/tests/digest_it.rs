//! Integration test for `pipeline::digest_summary`.
//!
//! Seeds listings + transitions in known states across multiple sources,
//! calls `digest_summary`, and asserts the report counts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use chrono::Duration;

use careerai_core::state::ListingState;
use careerai_db::{models::*, pool_from_path, queries};
use careerai_pipeline as pipeline;

fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
}

async fn seed_listing_in_state(
    pool: &careerai_db::SqlitePool,
    source: &str,
    external_id: &str,
    final_state: ListingState,
) -> String {
    let (id, _) = queries::insert_or_ignore(
        pool,
        &NewListing {
            source: source.into(),
            external_id: external_id.into(),
            title: "Test Role".into(),
            company: "Test Co".into(),
            location: Some("Remote".into()),
            url: format!("https://example.com/{external_id}"),
            description: "Description".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    // Walk the state machine through whatever steps are needed to reach final_state.
    // Each transition appends an `events` row.
    let path: &[ListingState] = match final_state {
        ListingState::Discovered => &[],
        ListingState::FilteredOut => &[ListingState::FilteredOut],
        ListingState::Shortlisted => &[ListingState::Shortlisted],
        ListingState::Tailored => &[ListingState::Shortlisted, ListingState::Tailored],
        ListingState::Rendered => &[
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
        ],
        ListingState::Prepared => &[
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
            ListingState::Prepared,
        ],
        ListingState::Drafted => &[
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
            ListingState::Drafted,
        ],
        ListingState::Submitted => &[
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
            ListingState::Submitted,
        ],
        ListingState::Failed => &[
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
            ListingState::Failed,
        ],
        ListingState::Skipped => &[ListingState::Shortlisted, ListingState::Skipped],
        ListingState::Responded => &[
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
            ListingState::Submitted,
            ListingState::Responded,
        ],
    };
    for to in path {
        queries::transition(pool, &id, *to, None).await.unwrap();
    }
    id
}

#[tokio::test]
async fn digest_summary_counts_transitions_in_window() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let pool = pool_from_path(&tmp.path().join("data/careerai.sqlite"))
        .await
        .unwrap();

    // Seed a spread of listings across sources and final states.
    seed_listing_in_state(&pool, "greenhouse", "gh-1", ListingState::Discovered).await;
    seed_listing_in_state(&pool, "greenhouse", "gh-2", ListingState::Shortlisted).await;
    seed_listing_in_state(&pool, "greenhouse", "gh-3", ListingState::Submitted).await;

    seed_listing_in_state(&pool, "lever", "lever-1", ListingState::Shortlisted).await;
    seed_listing_in_state(&pool, "lever", "lever-2", ListingState::Failed).await;

    seed_listing_in_state(&pool, "linkedin", "li-1", ListingState::Drafted).await;
    seed_listing_in_state(&pool, "linkedin", "li-2", ListingState::Responded).await;

    drop(pool);

    let report = pipeline::digest_summary(tmp.path(), Duration::hours(24))
        .await
        .expect("digest_summary should succeed");

    // 7 listings inserted → 7 'discovered' events.
    assert_eq!(report.discovered, 7, "discovered count");
    // Two transitioned through Shortlisted only; three through (Shortlisted +
    // Tailored + Rendered + ...) means Shortlisted events = 2 + 3 + 1 + 1 = ...
    // gh-2 → S; gh-3 → S T R Submitted; lever-1 → S; lever-2 → S T R Failed;
    // li-1 → S T R Drafted; li-2 → S T R Submitted Responded
    // = 6 distinct listings reaching Shortlisted (every state except Discovered).
    assert_eq!(report.shortlisted, 6, "shortlisted count");
    assert_eq!(report.drafted, 1, "drafted count (li-1)");
    // gh-3 + li-2 reached Submitted.
    assert_eq!(report.submitted, 2, "submitted count");
    assert_eq!(report.failed, 1, "failed count (lever-2)");
    assert_eq!(report.responded, 1, "responded count (li-2)");

    // Per-source: count of distinct listings with any activity.
    assert_eq!(
        report.per_source.get("greenhouse").map(|c| c.total),
        Some(3)
    );
    assert_eq!(report.per_source.get("lever").map(|c| c.total), Some(2));
    assert_eq!(report.per_source.get("linkedin").map(|c| c.total), Some(2));

    // last_tick must be Some — every transition wrote an event.
    assert!(report.last_tick.is_some(), "last_tick should be set");

    // No cookie warnings without keyring access in this test.
    assert!(report.cookie_warnings.is_empty(), "no warnings expected");
}

#[tokio::test]
async fn digest_summary_empty_db_returns_zero_counts() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());
    // Touch the DB so it exists.
    let pool = pool_from_path(&tmp.path().join("data/careerai.sqlite"))
        .await
        .unwrap();
    drop(pool);

    let report = pipeline::digest_summary(tmp.path(), Duration::hours(24))
        .await
        .expect("empty-DB digest must succeed");

    assert_eq!(report.discovered, 0);
    assert_eq!(report.matched, 0);
    assert_eq!(report.shortlisted, 0);
    assert_eq!(report.drafted, 0);
    assert_eq!(report.submitted, 0);
    assert_eq!(report.failed, 0);
    assert_eq!(report.responded, 0);
    assert!(report.per_source.is_empty());
    assert!(report.last_tick.is_none());
    assert!(report.cookie_warnings.is_empty());
}

#[tokio::test]
async fn digest_summary_window_excludes_old_events() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let pool = pool_from_path(&tmp.path().join("data/careerai.sqlite"))
        .await
        .unwrap();
    let id = seed_listing_in_state(&pool, "greenhouse", "gh-old", ListingState::Submitted).await;
    // Backdate every event for this listing to 48h ago — outside a 24h window.
    sqlx::query("UPDATE events SET created_at = datetime('now', '-48 hours') WHERE listing_id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    drop(pool);

    let report = pipeline::digest_summary(tmp.path(), Duration::hours(24))
        .await
        .unwrap();

    assert_eq!(
        report.discovered, 0,
        "old events outside the window must be excluded"
    );
    assert_eq!(report.submitted, 0);
}
