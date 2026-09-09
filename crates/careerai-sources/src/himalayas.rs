//! Himalayas API.
//!
//! Endpoint: `GET {base}/api/jobs`
//! Returns JSON with a `jobs` array of job objects.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://himalayas.app";

#[derive(Debug)]
pub struct HimalayasSource {
    base_url: String,
    http: Client,
}

impl Default for HimalayasSource {
    fn default() -> Self {
        #[allow(clippy::expect_used)]
        let http = Client::builder()
            .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
            .build()
            .expect("build reqwest client");
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
        }
    }
}

impl HimalayasSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for HimalayasSource {
    fn name(&self) -> &'static str {
        "himalayas"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!("{}/api/jobs", self.base_url.trim_end_matches('/'));
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
        let envelope: ResponseEnvelope = resp.json().await?;
        Ok(envelope
            .jobs
            .into_iter()
            .filter_map(|i| {
                let id = i.id?;
                let title = i.title?;
                let url = i.url?;
                if title.trim().is_empty() || url.trim().is_empty() {
                    return None;
                }
                Some(RawListing {
                    source: "himalayas".to_string(),
                    external_id: id,
                    title,
                    company: i.company.unwrap_or_default(),
                    location: i.location,
                    url,
                    description: i.description.map(|d| html_to_text(&d)).unwrap_or_default(),
                    raw_json: None,
                })
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct ResponseEnvelope {
    #[serde(default)]
    jobs: Vec<RawItem>,
}

#[derive(Debug, Deserialize)]
struct RawItem {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn parses_json_feed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [
                    {
                        "id": "abc-123",
                        "title": "Full Stack Engineer",
                        "company": "CloudCo",
                        "location": "Remote (Asia)",
                        "url": "https://himalayas.app/jobs/abc-123",
                        "description": "<p>Build web apps with Rust + React.</p>"
                    },
                    {
                        "id": "def-456",
                        "title": "DevOps Engineer",
                        "company": "InfraOps",
                        "location": "Remote (Worldwide)",
                        "url": "https://himalayas.app/jobs/def-456",
                        "description": "<p>Manage Kubernetes clusters.</p>"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let source = HimalayasSource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();

        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].external_id, "abc-123");
        assert_eq!(listings[0].title, "Full Stack Engineer");
        assert_eq!(listings[0].company, "CloudCo");
        assert!(listings[0].description.contains("Rust + React"));
        assert_eq!(listings[1].company, "InfraOps");
    }

    #[tokio::test]
    async fn skips_entries_without_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [
                    {"title": "No ID", "url": "https://example.com"},
                    {"id": "x", "title": "Valid", "url": "https://example.com/x"}
                ]
            })))
            .mount(&server)
            .await;

        let source = HimalayasSource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "x");
    }
}
