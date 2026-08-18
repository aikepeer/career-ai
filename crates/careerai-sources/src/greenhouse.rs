//! Greenhouse public board API.
//!
//! Endpoint: `GET {base}/v1/boards/{company}/jobs?content=true`
//! Docs: <https://developers.greenhouse.io/job-board.html>
//!
//! One instance per company slug. The orchestrator creates a list from
//! `config.sources.greenhouse.companies` and calls each in turn.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://boards-api.greenhouse.io";

#[derive(Debug)]
pub struct GreenhouseSource {
    company: String,
    base_url: String,
    http: Client,
}

impl GreenhouseSource {
    #[must_use]
    pub fn new(company: impl Into<String>) -> Self {
        Self {
            company: company.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            http: Client::new(),
        }
    }

    /// Override the base URL (tests point at wiremock).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for GreenhouseSource {
    fn name(&self) -> &'static str {
        "greenhouse"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!(
            "{}/v1/boards/{}/jobs?content=true",
            self.base_url.trim_end_matches('/'),
            self.company,
        );
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body: JobsPayload = resp.json().await?;
        Ok(body
            .jobs
            .into_iter()
            .map(|j| RawListing {
                source: "greenhouse".to_string(),
                external_id: j.id.to_string(),
                title: j.title,
                company: self.company.clone(),
                location: j.location.map(|l| l.name),
                url: j.absolute_url,
                description: html_to_text(&j.content.unwrap_or_default()),
                raw_json: None,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct JobsPayload {
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    id: i64,
    title: String,
    #[serde(default)]
    location: Option<JobLocation>,
    absolute_url: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JobLocation {
    name: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn discovers_listings_against_mock() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "jobs": [
                {
                    "id": 12345,
                    "title": "Senior ML Engineer",
                    "location": {"name": "Remote"},
                    "absolute_url": "https://boards.greenhouse.io/acme/jobs/12345",
                    "content": "<p>Build <strong>LLM</strong> apps &amp; robots.</p>"
                }
            ]
        });
        Mock::given(method("GET"))
            .and(path("/v1/boards/acme/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let src = GreenhouseSource::new("acme").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        let l = &listings[0];
        assert_eq!(l.source, "greenhouse");
        assert_eq!(l.external_id, "12345");
        assert_eq!(l.title, "Senior ML Engineer");
        assert_eq!(l.company, "acme");
        assert_eq!(l.location.as_deref(), Some("Remote"));
        assert_eq!(l.description, "Build LLM apps & robots.");
    }

    #[tokio::test]
    async fn http_error_surfaces_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let src = GreenhouseSource::new("bogus").with_base_url(server.uri());
        let err = src.discover().await.unwrap_err();
        assert!(matches!(err, SourceError::HttpStatus { status: 404, .. }));
    }
}
