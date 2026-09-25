//! Standalone test: verify migration 0010_quality_scores.sql applies cleanly
//! and the table has the expected schema.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_db::models::{NewApplication, NewListing};
use careerai_db::pool::pool_in_memory;
use careerai_db::queries;

#[tokio::test]
async fn migration_0010_quality_scores_applies_cleanly() {
    let pool = pool_in_memory().await.unwrap();

    // Verify the table exists and has the expected columns.
    let cols: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, type FROM pragma_table_info('application_quality_scores') ORDER BY cid",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let col_names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        col_names.contains(&"id"),
        "missing id column: {col_names:?}"
    );
    assert!(
        col_names.contains(&"application_id"),
        "missing application_id: {col_names:?}"
    );
    assert!(
        col_names.contains(&"overall"),
        "missing overall: {col_names:?}"
    );
    assert!(
        col_names.contains(&"jd_relevance"),
        "missing jd_relevance: {col_names:?}"
    );
    assert!(
        col_names.contains(&"skill_coverage"),
        "missing skill_coverage: {col_names:?}"
    );
    assert!(
        col_names.contains(&"cover_letter_depth"),
        "missing cover_letter_depth: {col_names:?}"
    );
    assert!(
        col_names.contains(&"bullet_density"),
        "missing bullet_density: {col_names:?}"
    );
    assert!(
        col_names.contains(&"recommendations"),
        "missing recommendations: {col_names:?}"
    );
    assert!(
        col_names.contains(&"created_at"),
        "missing created_at: {col_names:?}"
    );

    // Create a listing + application to satisfy the FK on application_quality_scores.
    let (listing_id, _) = queries::insert_or_ignore(
        &pool,
        &NewListing {
            source: "test".into(),
            external_id: "mig-1".into(),
            title: "Engineer".into(),
            company: "TestCo".into(),
            location: None,
            url: "https://example.com".into(),
            description: "Test role".into(),
            raw_json: None,
        },
    )
    .await
    .unwrap();

    let app = queries::create_application(
        &pool,
        &NewApplication {
            listing_id: listing_id.clone(),
            profile_hash: "sha256:mig".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "mock".into(),
        },
    )
    .await
    .unwrap();

    // Verify insert + select works via the store_quality_score query function.
    let row_id = queries::store_quality_score(&pool, &app.id, 0.85, 0.9, 0.8, 0.7, 0.6, "[]")
        .await
        .unwrap();
    assert!(row_id > 0, "inserted row should have a positive id");

    // Verify fetch returns the stored values.
    let fetched = queries::fetch_quality_score(&pool, &app.id)
        .await
        .unwrap()
        .expect("quality score should be fetchable");
    assert!((fetched.overall - 0.85).abs() < 1e-6);
    assert!((fetched.jd_relevance - 0.9).abs() < 1e-6);
    assert!((fetched.skill_coverage - 0.8).abs() < 1e-6);
    assert!((fetched.cover_letter_depth - 0.7).abs() < 1e-6);
    assert!((fetched.bullet_density - 0.6).abs() < 1e-6);

    println!("PASS: migration 0010 creates table with columns: {col_names:?}");
}
