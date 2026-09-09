//! Arbeitnow API.
//!
//! Endpoint: `GET {base}/api/v1/jobs`
//! Returns JSON with a `data` array of job objects.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://www.arbeitnow.com";

#[derive(Debug)]
pub struct ArbeitnowSource {
    base_url: String,
    http: Client,
}

impl Default for ArbeitnowSource {
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

impl ArbeitnowSource {
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
impl Source for ArbeitnowSource {
    fn name(&self) -> &'static str {
        "arbeitnow"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!("{}/api/v1/jobs", self.base_url.trim_end_matches('/'));
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
            .data
            .into_iter()
            .filter_map(|i| {
                let slug = i.slug?;
                let title = i.title?;
                if title.trim().is_empty() {
                    return None;
                }
                let url = i
                    .url
                    .unwrap_or_else(|| format!("https://www.arbeitnow.com/jobs/{slug}"));
                Some(RawListing {
                    source: "arbeitnow".to_string(),
                    external_id: slug,
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
    data: Vec<RawItem>,
}

#[derive(Debug, Deserialize)]
struct RawItem {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "company_name")]
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
            .and(path("/api/v1/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {
                        "slug": "rust-dev-berlin",
                        "title": "Rust Developer",
                        "company_name": "GermanTech GmbH",
                        "location": "Berlin, Germany",
                        "url": "https://www.arbeitnow.com/jobs/rust-dev-berlin",
                        "description": "<p>Backend development in Rust.</p>"
                    },
                    {
                        "slug": "ml-engineer-munich",
                        "title": "ML Engineer",
                        "company_name": "AI Munich",
                        "location": "Munich, Germany",
                        "description": "<p>Build ML pipelines.</p>"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let source = ArbeitnowSource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();

        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].external_id, "rust-dev-berlin");
        assert_eq!(listings[0].title, "Rust Developer");
        assert_eq!(listings[0].company, "GermanTech GmbH");
        assert!(listings[0].description.contains("Backend development"));
        // Second item has no url — should fall back to constructed URL
        assert_eq!(
            listings[1].url,
            "https://www.arbeitnow.com/jobs/ml-engineer-munich"
        );
    }

    #[tokio::test]
    async fn handles_empty_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": []
            })))
            .mount(&server)
            .await;

        let source = ArbeitnowSource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();
        assert!(listings.is_empty());
    }
}
