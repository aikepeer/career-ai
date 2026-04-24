//! RemoteOK API.
//!
//! Endpoint: `GET {base}/api`
//! The first array element is a legal notice, NOT a listing. We skip
//! entries that lack an `id` field to filter it out defensively.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use tracing;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://remoteok.com";

#[derive(Debug)]
pub struct RemoteOkSource {
    base_url: String,
    http: Client,
}

impl Default for RemoteOkSource {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            // RemoteOK rejects requests without a UA header.  Building with a
            // custom UA is not expected to fail; log loudly if it does so the
            // operator knows the UA will be missing (may cause 403s).
            http: Client::builder()
                .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
                .build()
                .unwrap_or_else(|e| {
                    tracing::error!(
                        error = %e,
                        "failed to build RemoteOK HTTP client with UA; \
                         falling back to plain client — requests may be blocked"
                    );
                    Client::new()
                }),
        }
    }
}

impl RemoteOkSource {
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
impl Source for RemoteOkSource {
    fn name(&self) -> &'static str {
        "remoteok"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!("{}/api", self.base_url.trim_end_matches('/'));
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let items: Vec<RawItem> = resp.json().await?;
        Ok(items
            .into_iter()
            .filter_map(|i| {
                let id = i.id?;
                Some(RawListing {
                    source: "remoteok".to_string(),
                    external_id: id,
                    title: i.position.unwrap_or_default(),
                    company: i.company.unwrap_or_default(),
                    location: i.location,
                    url: i.url.unwrap_or_default(),
                    description: i.description.map(|d| html_to_text(&d)).unwrap_or_default(),
                    raw_json: None,
                })
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct RawItem {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    position: Option<String>,
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
    async fn drops_legal_notice_element() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"legal": "Data provided by remoteok.com, ..."},
                {
                    "id": "abc",
                    "position": "Embedded Engineer",
                    "company": "Acme Robotics",
                    "location": "Remote",
                    "url": "https://remoteok.com/l/abc",
                    "description": "<p>ROS2, Rust, real-time control.</p>"
                }
            ])))
            .mount(&server)
            .await;

        let src = RemoteOkSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "abc");
        assert_eq!(listings[0].description, "ROS2, Rust, real-time control.");
    }
}
