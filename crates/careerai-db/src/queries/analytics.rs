//! Analytics & JD Intelligence Queries.
//!
//! Maintains historical correlation between Job Descriptions, Tailored Resumes,
//! and Cover Letters to identify market skill trends and conversion rates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::Result;

/// Comprehensive historical application record joining listing, application, and payload.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApplicationIntelligenceRecord {
    pub listing_id: String,
    pub application_id: String,
    pub title: String,
    pub company: String,
    pub source: String,
    pub jd_description: String,
    pub cover_letter_text: String,
    pub resume_view_json: String,
    pub application_state: String,
    pub created_at: DateTime<Utc>,
}

/// Fetch historical application records for trend and penetration analytics.
pub async fn query_intelligence_records(
    pool: &SqlitePool,
    limit: u32,
    offset: u32,
) -> Result<Vec<ApplicationIntelligenceRecord>> {
    let rows = sqlx::query_as(
        "SELECT l.id AS listing_id,
                a.id AS application_id,
                l.title AS title,
                l.company AS company,
                l.source AS source,
                l.description AS jd_description,
                p.cover_letter_text AS cover_letter_text,
                p.resume_view_json AS resume_view_json,
                a.state AS application_state,
                a.created_at AS created_at
         FROM applications a
         JOIN listings l ON a.listing_id = l.id
         JOIN application_payloads p ON a.id = p.application_id
         ORDER BY a.created_at DESC
         LIMIT ? OFFSET ?",
    )
    .bind(i64::from(limit))
    .bind(i64::from(offset))
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Aggregates submission conversion performance by source.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SourcePerformance {
    pub source: String,
    pub total_discovered: i64,
    pub total_submitted: i64,
}

pub async fn query_source_performance(pool: &SqlitePool) -> Result<Vec<SourcePerformance>> {
    let rows = sqlx::query_as(
        "SELECT l.source AS source,
                COUNT(DISTINCT l.id) AS total_discovered,
                COUNT(DISTINCT CASE WHEN a.state = 'submitted' THEN a.id END) AS total_submitted
         FROM listings l
         LEFT JOIN applications a ON l.id = a.listing_id
         GROUP BY l.source
         ORDER BY total_discovered DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::models::{NewApplication, NewListing};
    use crate::pool::pool_in_memory;
    use crate::queries::{applications, listings, payloads};

    #[tokio::test]
    async fn test_query_intelligence_records() {
        let pool = pool_in_memory().await.unwrap();

        let l = NewListing {
            source: "greenhouse".into(),
            external_id: "job-101".into(),
            title: "Rust Lead".into(),
            company: "Tech Robotics".into(),
            location: Some("Remote".into()),
            url: "https://example.com".into(),
            description: "Build low-latency Rust robotics controllers.".into(),
            raw_json: None,
        };
        let (listing_id, _) = listings::insert_or_ignore(&pool, &l).await.unwrap();

        let app = NewApplication {
            listing_id: listing_id.clone(),
            profile_hash: "hash123".into(),
            prompt_version: "v1".into(),
            llm_model: "local".into(),
        };
        let app_row = applications::create_application(&pool, &app).await.unwrap();

        payloads::write_payload(
            &pool,
            &app_row.id,
            "{\"skills\":[\"Rust\"]}",
            "Dear Tech Robotics...",
            "{}",
        )
        .await
        .unwrap();

        let records = query_intelligence_records(&pool, 10, 0).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "Rust Lead");
        assert_eq!(records[0].company, "Tech Robotics");
        assert!(records[0].cover_letter_text.contains("Tech Robotics"));

        let perf = query_source_performance(&pool).await.unwrap();
        assert_eq!(perf.len(), 1);
        assert_eq!(perf[0].source, "greenhouse");
        assert_eq!(perf[0].total_discovered, 1);
    }
}
