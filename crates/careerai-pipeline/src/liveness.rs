//! Liveness checks — verify shortlisted listings are still open before
//! the pipeline spends LLM tokens tailoring them (ported from
//! career-ops `check-liveness.mjs`).
//!
//! Zero-token by design: ATS APIs 404 a closed posting, so a HEAD/GET
//! against the source's single-posting endpoint tells us whether the
//! job is still open without an LLM call. Aggregator sources (RemoteOK,
//! Remotive, freehire) have no per-posting API and are reported as
//! `unknown` — never acted on.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::info;

use careerai_core::state::ListingState;
use careerai_db::queries;

use crate::open_pool;

/// One liveness verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    Closed,
    Unknown,
}

impl Liveness {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
        }
    }
}

/// Liveness verdict + the listing it belongs to.
#[derive(Debug, Clone)]
pub struct LivenessRow {
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub source: String,
    pub url: String,
    pub verdict: Liveness,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // reserved for the report summary in `careerai liveness`
pub struct LivenessReport {
    pub checked: usize,
    pub alive: usize,
    pub closed: usize,
    pub unknown: usize,
}

/// Check all shortlisted listings (optionally one source) against their
/// board's single-posting endpoint. Closed listings are NOT transitioned
/// here — the operator reviews the report (`careerai liveness`) and
/// decides; automation touching state belongs in the pipeline proper.
pub async fn check_liveness(root: &Path, source: Option<&str>) -> Result<Vec<LivenessRow>> {
    let pool = open_pool(root).await?;
    let listings = queries::list_by_state(&pool, ListingState::Shortlisted, 10_000)
        .await
        .context("list shortlisted")?;

    let mut report = Vec::new();
    for l in listings {
        if let Some(want) = source {
            if l.source != want {
                continue;
            }
        }
        let verdict = liveness_for(&l.source, &l.company, &l.external_id).await;
        info!(
            target = "liveness",
            listing_id = %l.id,
            source = %l.source,
            verdict = verdict.label(),
            "liveness probe"
        );
        report.push(LivenessRow {
            listing_id: l.id,
            title: l.title,
            company: l.company,
            source: l.source,
            url: l.url,
            verdict,
        });
    }
    Ok(report)
}

/// Probe one posting. Aggregator sources have no per-posting API →
/// `Unknown` without a network call.
async fn liveness_for(source: &str, company: &str, external_id: &str) -> Liveness {
    let Some(endpoint) = single_posting_endpoint(source, company, external_id) else {
        return Liveness::Unknown;
    };
    match probe_endpoint(&endpoint).await {
        Ok(true) => Liveness::Alive,
        Ok(false) => Liveness::Closed,
        Err(_) => Liveness::Unknown,
    }
}

/// The single-posting API URL for a source, when one exists.
fn single_posting_endpoint(source: &str, company: &str, external_id: &str) -> Option<String> {
    match source {
        // Greenhouse: `GET /v1/boards/{company}/jobs/{id}` — 404 when closed.
        "greenhouse" => Some(format!(
            "https://boards-api.greenhouse.io/v1/boards/{company}/jobs/{external_id}"
        )),
        // Lever: `GET /v0/postings/{company}/{id}?mode=json` — 404 when closed.
        "lever" => Some(format!(
            "https://api.lever.co/v0/postings/{company}/{external_id}?mode=json"
        )),
        // Teamtailor: JSON Feed 1.1 — membership check is cheaper: fetch
        // the board feed and look the id up. (Feed URLs are stable.)
        _ => None,
    }
}

/// GET the endpoint; `Ok(true)` = 200 (alive), `Ok(false)` = 404 (closed).
/// Any transport error or other status → `Err` (uncertain).
async fn probe_endpoint(endpoint: &str) -> Result<bool> {
    #[allow(clippy::expect_used)] // static UA — same pattern as adapters
    let client = reqwest::Client::builder()
        .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
        .build()
        .expect("build reqwest client");
    let resp = client
        .get(endpoint)
        .timeout(Duration::from_secs(15))
        .send()
        .await?;
    match resp.status().as_u16() {
        200 => Ok(true),
        404 => Ok(false),
        code => {
            tracing::warn!(endpoint, code, "liveness probe unexpected status");
            Err(anyhow::anyhow!("unexpected status {code}"))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn greenhouse_200_is_alive_404_is_closed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/boards/acme/jobs/123"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/boards/acme/jobs/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string(""))
            .mount(&server)
            .await;

        let alive = probe_endpoint(&format!("{}/v1/boards/acme/jobs/123", server.uri()))
            .await
            .unwrap();
        assert!(alive);
        let closed = probe_endpoint(&format!("{}/v1/boards/acme/jobs/999", server.uri()))
            .await
            .unwrap();
        assert!(!closed);
    }

    #[tokio::test]
    async fn transport_error_is_unknown_not_closed() {
        // Nothing mounted — connection refused.
        let server = MockServer::start().await;
        drop(server);
        let err = probe_endpoint("http://127.0.0.1:1/x").await;
        assert!(err.is_err());
    }

    #[test]
    fn aggregator_sources_have_no_endpoint() {
        for src in ["remoteok", "remotive", "freehire", "indeed_rss"] {
            assert!(
                single_posting_endpoint(src, "acme", "x1").is_none(),
                "{src} must not have a per-posting endpoint"
            );
        }
        assert!(single_posting_endpoint("greenhouse", "acme", "123")
            .unwrap()
            .contains("/v1/boards/acme/jobs/123"));
        assert!(single_posting_endpoint("lever", "acme", "456")
            .unwrap()
            .contains("/v0/postings/acme/456"));
    }
}
