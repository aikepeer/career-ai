//! Pattern analysis (ported from career-ops `analyze-patterns.mjs` and
//! `detect-reposts.mjs`).
//!
//! Thin orchestration over `careerai_db::queries::patterns`. Returns
//! serialisable reports the CLI renders as tables.

use anyhow::Result;
use careerai_db::queries::patterns as q;
use serde::Serialize;

/// Aggregate pattern report combining all sub-analyses.
#[derive(Debug, Serialize)]
pub struct PatternReport {
    pub reposts: Vec<q::Repost>,
    pub funnel: Vec<q::FunnelVelocity>,
    pub advance_rates: Vec<q::AdvanceRate>,
    pub rejections: Vec<q::RejectionLatency>,
}

/// Run all pattern analyses against the DB.
pub async fn analyze_patterns(pool: &sqlx::SqlitePool) -> Result<PatternReport> {
    let reposts = q::detect_reposts(pool).await?;
    let funnel = q::funnel_velocity(pool).await?;
    let advance_rates = q::advance_rates(pool).await?;
    let rejections = q::rejection_latencies(pool).await?;
    Ok(PatternReport {
        reposts,
        funnel,
        advance_rates,
        rejections,
    })
}
