//! Integration test: salary extraction + storage fires during discovery.
//!
//! The `discover_with_sources` function calls `extract_salary_range` on
//! each new listing's description and, if a salary pattern is found, calls
//! `store_salary_range` to persist it. This test verifies those two
//! functions work end-to-end with a real SQLite DB — the exact chain
//! executed inside the `Ok((id, true))` arm of `insert_or_ignore`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_possible_wrap)]

use careerai_db::models::NewListing;
use careerai_db::{pool::pool_in_memory, queries};

#[tokio::test]
async fn salary_extraction_persists_to_salary_ranges_table() {
    let pool = pool_in_memory().await.unwrap();

    // Seed a listing whose description contains a salary pattern.
    let new = NewListing {
        source: "greenhouse".into(),
        external_id: "sal-1".into(),
        title: "Senior ML Engineer".into(),
        company: "Acme Robotics".into(),
        location: Some("Remote".into()),
        url: "https://example.com/sal-1".into(),
        description: "Build LLM systems. Salary: $120,000 - $150,000 per year.".into(),
        raw_json: None,
    };
    let (listing_id, inserted) = queries::insert_or_ignore(&pool, &new).await.unwrap();
    assert!(inserted, "listing should be newly inserted");

    // Replicate the wiring in discover.rs: extract salary from JD text.
    let range = careerai_match::extract_salary_range(&new.description)
        .expect("salary pattern should be detected in description");

    // Persist to the salary_ranges table — same call as discover.rs.
    queries::store_salary_range(
        &pool,
        &listing_id,
        &new.company,
        &new.title,
        range.min.map(|v| v as i64),
        range.max.map(|v| v as i64),
        &range.currency,
        &range.period,
        Some(&range.raw_text),
    )
    .await
    .unwrap();

    // Verify the row exists with the expected values.
    let rows = queries::list_salary_ranges(&pool, 10).await.unwrap();
    assert_eq!(rows.len(), 1, "exactly one salary_ranges row should exist");
    assert_eq!(rows[0].listing_id, listing_id);
    assert_eq!(rows[0].company, "Acme Robotics");
    assert_eq!(rows[0].title, "Senior ML Engineer");
    assert_eq!(rows[0].min_salary, Some(120_000));
    assert_eq!(rows[0].max_salary, Some(150_000));
    assert_eq!(rows[0].currency, "USD");
}

#[tokio::test]
async fn salary_extraction_noop_when_no_salary_pattern() {
    let pool = pool_in_memory().await.unwrap();

    let new = NewListing {
        source: "greenhouse".into(),
        external_id: "sal-2".into(),
        title: "Backend Engineer".into(),
        company: "NoSalary Inc".into(),
        location: Some("Remote".into()),
        url: "https://example.com/sal-2".into(),
        description: "Build backend services. Competitive compensation.".into(),
        raw_json: None,
    };
    let (listing_id, _) = queries::insert_or_ignore(&pool, &new).await.unwrap();

    // No salary pattern → extract_salary_range returns None → no DB write.
    let range = careerai_match::extract_salary_range(&new.description);
    assert!(range.is_none(), "no salary pattern should yield None");

    let rows = queries::list_salary_ranges(&pool, 10).await.unwrap();
    assert!(
        rows.is_empty(),
        "no salary_ranges rows should exist when no pattern is found"
    );

    // listing_id is used to confirm the listing was inserted;
    // the salary table should still be empty.
    assert!(!listing_id.is_empty());
}
