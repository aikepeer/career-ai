//! Lever public postings API.
//!
//! Endpoint: `GET {base}/v0/postings/{company}?mode=json`
//! Docs: <https://help.lever.co/hc/en-us/articles/360046309631>

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://api.lever.co";

#[derive(Debug)]
pub struct LeverSource {
    company: String,
    base_url: String,
    http: Client,
}

impl LeverSource {
    #[must_use]
    pub fn new(company: impl Into<String>) -> Self {
        Self {
            company: company.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            http: Client::new(),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for LeverSource {
    fn name(&self) -> &'static str {
        "lever"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!(
            "{}/v0/postings/{}?mode=json",
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
        let postings: Vec<Posting> = resp.json().await?;
        Ok(postings
            .into_iter()
            .map(|p| {
                // Prefer the plaintext description; fall back to HTML strip.
                let description = if p.description_plain.is_empty() {
                    html_to_text(&p.description)
                } else {
                    p.description_plain
                };
                RawListing {
                    source: "lever".to_string(),
                    external_id: p.id,
                    title: p.text,
                    company: self.company.clone(),
                    location: p.categories.location,
                    url: p.hosted_url,
                    description,
                    raw_json: None,
                }
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct Posting {
    id: String,
    text: String,
    #[serde(default, rename = "description")]
    description: String,
    #[serde(default, rename = "descriptionPlain")]
    description_plain: String,
    #[serde(default)]
    categories: Categories,
    #[serde(rename = "hostedUrl")]
    hosted_url: String,
}

#[derive(Debug, Default, Deserialize)]
struct Categories {
    #[serde(default)]
    location: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn discovers_listings_with_plaintext_description() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "abc-123",
                "text": "Robotics Engineer",
                "descriptionPlain": "Build autonomous systems.",
                "description": "<p>Ignored in favor of plaintext.</p>",
                "categories": {"location": "Remote - India"},
                "hostedUrl": "https://jobs.lever.co/acme/abc-123"
            }
        ]);
        Mock::given(method("GET"))
            .and(path("/v0/postings/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let src = LeverSource::new("acme").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "abc-123");
        assert_eq!(listings[0].description, "Build autonomous systems.");
        assert_eq!(listings[0].location.as_deref(), Some("Remote - India"));
    }

    #[tokio::test]
    async fn falls_back_to_html_description_when_plain_empty() {
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {
                "id": "x",
                "text": "Title",
                "description": "<p>HTML body</p>",
                "categories": {},
                "hostedUrl": "https://jobs.lever.co/acme/x"
            }
        ]);
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let src = LeverSource::new("acme").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings[0].description, "HTML body");
    }
}
