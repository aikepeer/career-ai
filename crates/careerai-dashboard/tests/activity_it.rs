#![allow(clippy::expect_used)]

use careerai_dashboard::activity::summary_at;
use careerai_db::pool::pool_in_memory;
use chrono::{TimeZone, Utc};

#[tokio::test]
async fn activity_counts_real_submissions_once_and_uses_monday_utc_week() {
    let pool = pool_in_memory().await.expect("database");
    sqlx::query("INSERT INTO listings (id, source, external_id, title, company, url, description) VALUES ('one', 'test', 'one', 'Engineer', 'Acme', 'https://example.com', '')")
        .execute(&pool).await.expect("listing");
    for (state, date) in [
        ("rendered", "2026-09-07T10:00:00Z"),
        ("submitted", "2026-09-06T10:00:00Z"),
        ("submitted", "2026-09-07T11:00:00Z"),
        ("submitted", "2026-09-08T11:00:00Z"),
        ("responded", "2026-09-08T12:00:00Z"),
        ("submitted", "2026-09-09T11:00:00Z"),
    ] {
        sqlx::query("INSERT INTO events (listing_id, to_state, created_at) VALUES ('one', ?, ?)")
            .bind(state)
            .bind(date)
            .execute(&pool)
            .await
            .expect("event");
    }
    let now = Utc
        .with_ymd_and_hms(2026, 9, 8, 13, 0, 0)
        .single()
        .expect("date");
    let summary = summary_at(&pool, now).await.expect("summary");
    assert_eq!(summary.week_start, "2026-09-07");
    assert_eq!(summary.submitted_this_week, 1);
    assert_eq!(summary.days.len(), 7);
    assert_eq!(summary.days[0].date, "2026-09-02");
    assert_eq!(summary.days[6].date, "2026-09-08");
    assert_eq!(summary.days[6].submitted, 1);
    assert_eq!(summary.days[6].responded, 1);
    assert_eq!(summary.days[0].submitted, 0);
}

#[tokio::test]
async fn empty_activity_returns_seven_zero_days_and_database_errors_propagate() {
    let pool = pool_in_memory().await.expect("database");
    let summary = summary_at(&pool, Utc::now()).await.expect("summary");
    assert_eq!(summary.submitted_this_week, 0);
    assert_eq!(summary.days.len(), 7);
    assert!(summary
        .days
        .iter()
        .all(|day| day.submitted == 0 && day.responded == 0));
    pool.close().await;
    assert!(summary_at(&pool, Utc::now()).await.is_err());
}
