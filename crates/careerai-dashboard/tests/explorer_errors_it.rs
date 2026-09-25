#![allow(clippy::expect_used)]

use careerai_dashboard::details::fetch_discovered_explorer;
use careerai_db::pool::pool_in_memory;

#[tokio::test]
async fn explorer_reports_database_failure_instead_of_empty_success() {
    let pool = pool_in_memory().await.expect("database");
    pool.close().await;
    assert!(fetch_discovered_explorer(&pool, 10).await.is_err());
}

#[tokio::test]
async fn explorer_does_not_claim_unchecked_work_authorization() {
    let pool = pool_in_memory().await.expect("database");
    sqlx::query("INSERT INTO listings (id, source, external_id, title, company, url, description) VALUES ('one', 'test', 'one', 'Engineer', 'Acme', 'https://example.com', 'Build software')")
        .execute(&pool).await.expect("listing");
    let rows = fetch_discovered_explorer(&pool, 10)
        .await
        .expect("explorer");
    assert_eq!(rows[0].eligibility, "unknown");
}
