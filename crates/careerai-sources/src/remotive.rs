//! Remotive remote-jobs API.
//!
//! Endpoint: `GET {base}/api/remote-jobs[?category=<slug>]`
//! Docs: <https://remotive.com/api-documentation>

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://remotive.com";

#[derive(Debug)]
pub struct RemotiveSource {
    category: Option<String>,
    base_url: String,
    http: Client,
}

impl Default for RemotiveSource {
    fn default() -> Self {
        Self {
            category: Some("software-dev".to_string()),
            base_url: DEFAULT_BASE_URL.to_string(),
            http: Client::new(),
        }
    }
}

impl RemotiveSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for RemotiveSource {
    fn name(&self) -> &'static str {
        "remotive"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/api/remote-jobs");
        let mut req = self.http.get(&url).timeout(Duration::from_secs(30));
        if let Some(c) = &self.category {
            req = req.query(&[("category", c)]);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body: Payload = resp.json().await?;
        Ok(body
            .jobs
            .into_iter()
            .map(|j| RawListing {
                source: "remotive".to_string(),
                external_id: j.id.to_string(),
                title: j.title,
                company: j.company_name,
                location: Some(j.candidate_required_location).filter(|s| !s.is_empty()),
                url: j.url,
                description: html_to_text(&j.description),
                raw_json: None,
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct Payload {
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    id: i64,
    title: String,
    company_name: String,
    url: String,
    #[serde(default)]
    candidate_required_location: String,
    #[serde(default)]
    description: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn discovers_with_category_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/remote-jobs"))
            .and(query_param("category", "software-dev"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": 42,
                    "title": "LLM Infra Engineer",
                    "company_name": "Acme",
                    "url": "https://remotive.com/jobs/42",
                    "candidate_required_location": "Worldwide",
                    "description": "<p>Ship infra.</p>"
                }]
            })))
            .mount(&server)
            .await;

        let src = RemotiveSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "42");
        assert_eq!(listings[0].company, "Acme");
        assert_eq!(listings[0].description, "Ship infra.");
    }
}
