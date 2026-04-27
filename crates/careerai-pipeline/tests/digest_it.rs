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

    // Two filtered_out — proves the matched += branch fires for
    // filtered_out, not just shortlisted.
    seed_listing_in_state(&pool, "lever", "lever-3", ListingState::FilteredOut).await;
    seed_listing_in_state(&pool, "greenhouse", "gh-4", ListingState::FilteredOut).await;

    drop(pool);

    let report = pipeline::digest_summary(tmp.path(), Duration::hours(24))
        .await
        .expect("digest_summary should succeed");

    // 9 listings inserted → 9 'discovered' events.
    assert_eq!(report.discovered, 9, "discovered count");
    // Six listings reached Shortlisted (every non-FilteredOut, non-
    // pure-Discovered listing): gh-2, gh-3, lever-1, lever-2, li-1,
    // li-2 — that's 6. (gh-1 stays Discovered; gh-4 + lever-3 go to
    // FilteredOut without passing through Shortlisted.)
    assert_eq!(report.shortlisted, 6, "shortlisted count");
    // matched = shortlisted + filtered_out distinct listings
    //         = 6 + 2 = 8
    assert_eq!(
        report.matched, 8,
        "matched should include both shortlisted and filtered_out"
    );
    assert_eq!(report.drafted, 1, "drafted count (li-1)");
    // gh-3 + li-2 reached Submitted.
    assert_eq!(report.submitted, 2, "submitted count");
    assert_eq!(report.failed, 1, "failed count (lever-2)");
    assert_eq!(report.responded, 1, "responded count (li-2)");

    // Per-source: count of distinct listings with any activity.
    // greenhouse: gh-1, gh-2, gh-3, gh-4 = 4
    assert_eq!(
        report.per_source.get("greenhouse").map(|c| c.total),
        Some(4)
    );
    // lever: lever-1, lever-2, lever-3 = 3
    assert_eq!(report.per_source.get("lever").map(|c| c.total), Some(3));
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
    // Backdate every event for this listing to 48h ago — outside a 24h
    // window. Use the exact production timestamp format
    // (`strftime('%Y-%m-%dT%H:%M:%fZ')`) — events.created_at default
    // uses this shape, and the digest WHERE clause compares as TEXT.
    // A mismatched format (e.g. `datetime('now', '-48h')` which lacks
    // the `T` and `Z`) would lex-compare differently and could
    // accidentally include or exclude rows at the boundary.
    sqlx::query(
        "UPDATE events
         SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-48 hours')
         WHERE listing_id = ?",
    )
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

/// `listings.source` isn't lowercase-enforced by the schema. Discovery
/// adapters write lowercase, but a hand-edited row with `"LinkedIn"`
/// or `"LINKEDIN"` would otherwise produce three separate buckets in
/// `per_source`. The query must merge them via `LOWER()` (matching the
/// PR #14 convention in `list_drafted_linkedin`).
#[tokio::test]
async fn digest_summary_per_source_is_case_insensitive() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let pool = pool_from_path(&tmp.path().join("data/careerai.sqlite"))
        .await
        .unwrap();
    seed_listing_in_state(&pool, "linkedin", "li-lower", ListingState::Shortlisted).await;
    seed_listing_in_state(&pool, "LinkedIn", "li-mixed", ListingState::Shortlisted).await;
    seed_listing_in_state(&pool, "LINKEDIN", "li-upper", ListingState::Shortlisted).await;
    drop(pool);

    let report = pipeline::digest_summary(tmp.path(), Duration::hours(24))
        .await
        .unwrap();

    // All three rows must aggregate under a single lowercase bucket.
    assert_eq!(
        report.per_source.get("linkedin").map(|c| c.total),
        Some(3),
        "case variants must merge into a single 'linkedin' bucket"
    );
    assert!(
        !report.per_source.contains_key("LinkedIn"),
        "no separate mixed-case bucket"
    );
    assert!(
        !report.per_source.contains_key("LINKEDIN"),
        "no separate upper-case bucket"
    );
}
