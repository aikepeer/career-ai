//! F02: Verify migration 0012_match_reasons.sql applies cleanly and the
//! match_reasons query functions (upsert + fetch) work correctly.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_db::models::NewListing;
use careerai_db::pool::pool_in_memory;
use careerai_db::queries;

#[tokio::test]
async fn migration_0012_match_reasons_applies_and_upsert_works() {
    let pool = pool_in_memory().await.unwrap();

    // Verify the table exists and has the expected columns.
    let cols: Vec<(String, String)> =
        sqlx::query_as("SELECT name, type FROM pragma_table_info('match_reasons') ORDER BY cid")
            .fetch_all(&pool)
            .await
            .unwrap();

    let col_names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
    for expected in [
        "id",
        "listing_id",
        "score",
        "matched_keywords",
        "missing_keywords",
        "filter_reason",
        "legitimacy_tier",
        "legitimacy_score",
        "eligibility_note",
        "matched_at",
    ] {
        assert!(
            col_names.contains(&expected),
            "missing column {expected}: {col_names:?}"
        );
    }

    // Seed a listing to satisfy the FK.
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "fixture".into(),
            external_id: "mr-1".into(),
            title: "Rust Engineer".into(),
            company: "Acme".into(),
            location: None,
            url: "https://example.com".into(),
            description: "Build things in Rust.".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    // Upsert a match-reasons row.
    let row_id = queries::upsert_match_reasons(
        &pool,
        &listing_id,
        0.82,
        r#"["rust","tokio","async"]"#,
        r#"["grpc"]"#,
        None,
        Some("caution"),
        Some(0.45),
        Some("auth: unknown"),
    )
    .await
    .unwrap();
    assert!(row_id > 0);

    // Fetch and verify.
    let fetched = queries::fetch_match_reasons(&pool, &listing_id)
        .await
        .unwrap()
        .expect("match reasons should be fetchable");
    assert!((fetched.score - 0.82).abs() < 1e-6);
    assert_eq!(fetched.matched_keywords, r#"["rust","tokio","async"]"#);
    assert_eq!(fetched.missing_keywords, r#"["grpc"]"#);
    assert!(fetched.filter_reason.is_none());
    assert_eq!(fetched.legitimacy_tier.as_deref(), Some("caution"));
    assert!((fetched.legitimacy_score.unwrap() - 0.45).abs() < 1e-6);
    assert_eq!(fetched.eligibility_note.as_deref(), Some("auth: unknown"));

    // Upsert again (simulating a re-match) — should update, not duplicate.
    let row_id2 = queries::upsert_match_reasons(
        &pool,
        &listing_id,
        0.90,
        r#"["rust","tokio","async","grpc"]"#,
        "[]",
        None,
        Some("legit"),
        Some(0.92),
        None,
    )
    .await
    .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM match_reasons WHERE listing_id = ?")
        .bind(&listing_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "upsert should not duplicate rows");

    let updated = queries::fetch_match_reasons(&pool, &listing_id)
        .await
        .unwrap()
        .expect("updated match reasons should be fetchable");
    assert!((updated.score - 0.90).abs() < 1e-6);
    assert_eq!(updated.legitimacy_tier.as_deref(), Some("legit"));
    assert!(row_id2 > 0);

    println!("PASS: migration 0012 + match_reasons upsert/fetch verified");
}

#[tokio::test]
async fn match_reasons_fetch_returns_none_for_unknown_listing() {
    let pool = pool_in_memory().await.unwrap();
    let result = queries::fetch_match_reasons(&pool, "nonexistent")
        .await
        .unwrap();
    assert!(result.is_none());
}
