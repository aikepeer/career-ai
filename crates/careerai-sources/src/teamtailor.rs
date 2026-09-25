//! Teamtailor careers JSON feed.
//!
//! Endpoint: `GET {base}/jobs.json` — a [JSON Feed 1.1] document that
//! Teamtailor serves for every hosted careers board. Each item also carries
//! a schema.org `_jobposting` block with the hiring organization and
//! location, which is more structured than the feed body itself.
//!
//! One instance per company subdomain (e.g. `synmatchai` →
//! `https://synmatchai.teamtailor.com`).
//!
//! [JSON Feed 1.1]: https://www.jsonfeed.org/version/1.1/

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const SUBDOMAIN_SUFFIX: &str = "teamtailor.com";

#[derive(Debug)]
pub struct TeamtailorSource {
    company: String,
    base_url: String,
    http: Client,
}

impl TeamtailorSource {
    #[must_use]
    pub fn new(company: impl Into<String>) -> Self {
        let company = company.into();
        let base_url = format!("https://{company}.{SUBDOMAIN_SUFFIX}");
        Self {
            company,
            base_url,
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
impl Source for TeamtailorSource {
    fn name(&self) -> &'static str {
        "teamtailor"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!("{}/jobs.json", self.base_url.trim_end_matches('/'));
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
        let feed: Feed = resp.json().await?;
        let fallback_company = feed
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.company.clone());

        Ok(feed
            .items
            .into_iter()
            .filter_map(|item| {
                if item.id.trim().is_empty()
                    || item.title.trim().is_empty()
                    || item.url.trim().is_empty()
                {
                    return None;
                }
                let posting = item.jobposting.as_ref();
                let company = posting
                    .and_then(|p| p.hiring_organization.as_ref())
                    .and_then(|o| o.name.as_deref())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(&fallback_company);
                let location = posting.and_then(|p| extract_location(&p.job_location));
                let description = if item.content_html.trim().is_empty() {
                    posting
                        .and_then(|p| p.description.as_deref())
                        .map(html_to_text)
                        .unwrap_or_default()
                } else {
                    html_to_text(&item.content_html)
                };

                Some(RawListing {
                    source: "teamtailor".to_string(),
                    external_id: item.id,
                    title: item.title,
                    company: company.to_string(),
                    location,
                    url: item.url,
                    description,
                    raw_json: None,
                })
            })
            .collect())
    }
}

fn extract_location(places: &[Place]) -> Option<String> {
    let addr = places.first()?.address.as_ref()?;
    addr.locality
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| addr.region.as_deref().filter(|s| !s.trim().is_empty()))
        .or_else(|| addr.country.as_deref().filter(|s| !s.trim().is_empty()))
        .or_else(|| addr.street.as_deref().filter(|s| !s.trim().is_empty()))
        .map(str::to_string)
}

#[derive(Debug, Deserialize)]
struct Feed {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content_html: String,
    #[serde(default, rename = "_jobposting")]
    jobposting: Option<JobPosting>,
}

#[derive(Debug, Deserialize)]
struct JobPosting {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    hiring_organization: Option<Organization>,
    #[serde(default, rename = "jobLocation")]
    job_location: Vec<Place>,
}

#[derive(Debug, Deserialize)]
struct Organization {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Place {
    #[serde(default)]
    address: Option<Address>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Address {
    #[serde(default)]
    street: Option<String>,
    #[serde(default, rename = "addressLocality")]
    locality: Option<String>,
    #[serde(default, rename = "addressCountry")]
    country: Option<String>,
    #[serde(default, rename = "addressRegion")]
    region: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_feed() -> serde_json::Value {
        serde_json::json!({
            "version": "https://jsonfeed.org/version/1.1",
            "title": "Synmatch AI",
            "home_page_url": "https://synmatchai.teamtailor.com/jobs",
            "feed_url": "https://synmatchai.teamtailor.com/jobs.json",
            "items": [
                {
                    "id": "job-1",
                    "title": "Full Stack Engineer",
                    "url": "https://synmatchai.teamtailor.com/jobs/1-full-stack-engineer",
                    "content_html": "<p>Build <strong>Django</strong> + React apps.</p>",
                    "_jobposting": {
                        "hiringOrganization": { "name": "Synmatch AI" },
                        "jobLocation": [{
                            "@type": "Place",
                            "address": {
                                "streetAddress": "Europe",
                                "addressLocality": "European Union"
                            }
                        }]
                    }
                },
                {
                    "id": "job-2",
                    "title": "Account Executive",
                    "url": "https://synmatchai.teamtailor.com/jobs/2-account-executive",
                    "content_html": "<p>Drive sales.</p>",
                    "_jobposting": {
                        "hiringOrganization": { "name": "Synmatch AI" },
                        "jobLocation": [{
                            "@type": "Place",
                            "address": { "addressLocality": "Spain", "addressCountry": "ES" }
                        }]
                    }
                }
            ]
        })
    }

    #[tokio::test]
    async fn discovers_listings_against_mock() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jobs.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_feed()))
            .mount(&server)
            .await;

        let src = TeamtailorSource::new("synmatchai").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 2);

        let first = &listings[0];
        assert_eq!(first.source, "teamtailor");
        assert_eq!(first.external_id, "job-1");
        assert_eq!(first.title, "Full Stack Engineer");
        assert_eq!(first.company, "Synmatch AI");
        assert_eq!(first.location.as_deref(), Some("European Union"));
        assert_eq!(first.description, "Build Django + React apps.");

        // Falls back from locality to country when locality is absent.
        assert_eq!(listings[1].location.as_deref(), Some("Spain"));
    }

    #[tokio::test]
    async fn skips_items_missing_id_title_or_url() {
        let server = MockServer::start().await;
        let mut feed = sample_feed();
        feed["items"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "id": "", "title": "", "url": "" }));
        Mock::given(method("GET"))
            .and(path("/jobs.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(feed))
            .mount(&server)
            .await;

        let src = TeamtailorSource::new("synmatchai").with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 2, "empty item must be skipped");
    }
}
