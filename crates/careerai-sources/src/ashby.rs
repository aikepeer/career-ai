//! Ashby public posting API.
//!
//! Endpoint: `GET {base}/posting-api/job-board/{company}?includeCompensation=true`
//! Docs: <https://developers.ashbyhq.com/reference/jobboardlistjobpostings>
//!
//! Same shape as Greenhouse / Lever — one instance per company slug.
//! `careerai sources sync` populates `sources.ashby.companies` from a
//! seeded list of slugs that publish on Ashby.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://api.ashbyhq.com";

#[derive(Debug)]
pub struct AshbySource {
    company: String,
    base_url: String,
    http: Client,
}

impl AshbySource {
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
impl Source for AshbySource {
    fn name(&self) -> &'static str {
        "ashby"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!(
            "{}/posting-api/job-board/{}",
            self.base_url.trim_end_matches('/'),
            self.company,
        );
        let resp = self.http.get(&url).send().await?;
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
                source: "ashby".to_string(),
                external_id: j.id,
                title: j.title,
                company: self.company.clone(),
                location: Some(j.location),
                url: j.job_url,
                description: html_to_text(&j.description_html.unwrap_or_default()),
                raw_json: None,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct JobsPayload {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_field_names)] // upstream key names dictate this
struct Job {
    id: String,
    title: String,
    #[serde(default)]
    location: String,
    #[serde(rename = "jobUrl")]
    job_url: String,
    #[serde(default, rename = "descriptionHtml")]
    description_html: Option<String>,
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
                    "id": "abc-123",
                    "title": "Staff ML Engineer",
                    "location": "Remote",
                    "jobUrl": "https://jobs.ashbyhq.com/acme/abc-123",
                    "descriptionHtml": "<p>Build <strong>LLM</strong> agents.</p>"
                }
            ]
        });
        Mock::given(method("GET"))
            .and(path("/posting-api/job-board/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let src = AshbySource::new("acme").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        let l = &listings[0];
        assert_eq!(l.source, "ashby");
        assert_eq!(l.external_id, "abc-123");
        assert_eq!(l.title, "Staff ML Engineer");
        assert_eq!(l.company, "acme");
        assert_eq!(l.location.as_deref(), Some("Remote"));
        assert_eq!(l.description, "Build LLM agents.");
    }

    #[tokio::test]
    async fn http_error_surfaces_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let src = AshbySource::new("bogus").with_base_url(server.uri());
        let err = src.discover().await.unwrap_err();
        assert!(matches!(err, SourceError::HttpStatus { status: 404, .. }));
    }

    #[tokio::test]
    async fn empty_jobs_array_yields_no_listings() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"jobs": []});
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let src = AshbySource::new("acme").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert!(listings.is_empty());
    }
}
