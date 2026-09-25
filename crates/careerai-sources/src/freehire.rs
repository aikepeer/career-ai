//! FreeHire aggregator source.
//!
//! Endpoint: `GET {base}/api/v1/agent/jobs/search` — public, no auth, no
//! API key. FreeHire (`freehire.me`) normalizes postings from ~50 ATS
//! platforms into one schema and is tuned tech-first (`category`,
//! `seniority`, `skills` facets), which matches this project's AI/ML +
//! embedded/robotics niche. The backend is MIT-licensed and self-hostable
//! (`FREEHIRE_API_URL` equivalent: `base_url`), so pointing the adapter at
//! a local instance is a one-line config change.
//!
//! Response shape (verified live 2026-08-30): `{"data": [ ... ], "meta": {...}}`
//! with per-item `public_slug` (dedupe id), `title`, `company`,
//! `location`, `url`, HTML `description`, `posted_at`, and — when
//! enriched — `closed_at`, `skills`, `countries`/`regions`, `work_mode`.
//! Closed postings (`closed_at` set) are skipped, mirroring the upstream
//! skill's "report closed postings" behavior in the least surprising form
//! for an automated pipeline: they are not inserted.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://freehire.me";
const DEFAULT_LIMIT: usize = 25;

#[derive(Debug)]
pub struct FreehireSource {
    base_url: String,
    http: Client,
    query: String,
    limit: usize,
    /// `remote=remote` facet — restrict to fully-remote postings.
    remote_only: bool,
    /// `region=<codes>` facet (e.g. `eu`, `global`); pass-through, the
    /// upstream facet vocabulary owns the valid values.
    region: Option<String>,
    /// `jobage=<days>` — posted within N days.
    jobage: Option<u32>,
}

impl Default for FreehireSource {
    fn default() -> Self {
        #[allow(clippy::expect_used)]
        let http = Client::builder()
            .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
            .build()
            .expect("build reqwest client with static UA");
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
            query: String::new(),
            limit: DEFAULT_LIMIT,
            remote_only: false,
            region: None,
            jobage: None,
        }
    }
}

impl FreehireSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    #[must_use]
    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query = query.into();
        self
    }

    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    #[must_use]
    pub fn with_remote_only(mut self, remote_only: bool) -> Self {
        self.remote_only = remote_only;
        self
    }

    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    #[must_use]
    pub fn with_jobage(mut self, days: u32) -> Self {
        self.jobage = Some(days);
        self
    }
}

#[async_trait]
impl Source for FreehireSource {
    fn name(&self) -> &'static str {
        "freehire"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let mut req = self
            .http
            .get(format!(
                "{}/api/v1/agent/jobs/search",
                self.base_url.trim_end_matches('/')
            ))
            .timeout(Duration::from_secs(30));
        if !self.query.is_empty() {
            req = req.query(&[("q", self.query.as_str())]);
        }
        req = req.query(&[("limit", self.limit.to_string())]);
        if self.remote_only {
            req = req.query(&[("remote", "remote")]);
        }
        if let Some(region) = &self.region {
            req = req.query(&[("region", region.as_str())]);
        }
        if let Some(days) = self.jobage {
            req = req.query(&[("jobage", days.to_string())]);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let payload: SearchResponse = resp
            .json()
            .await
            .map_err(|e| SourceError::Parse(e.to_string()))?;

        let mut out = Vec::new();
        for item in payload.data {
            // Closed postings are still listed by the API (the upstream
            // CLI surfaces them with a flag); an automated pipeline must
            // not ingest them — they fail at submit time otherwise.
            if item.closed_at.is_some() {
                continue;
            }
            let title = item.title.trim().to_string();
            if title.is_empty() {
                continue;
            }
            let url = item
                .url
                .clone()
                .unwrap_or_else(|| format!("https://freehire.me/jobs/{}", item.public_slug));
            out.push(RawListing {
                source: "freehire".to_string(),
                external_id: item.public_slug,
                title,
                company: item.company.unwrap_or_default(),
                location: item.location,
                url,
                description: item
                    .description
                    .map(|d| html_to_text(&d))
                    .unwrap_or_default(),
                raw_json: None,
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    data: Vec<SearchItem>,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    /// FreeHire `public_slug` — stable dedupe id (`<title>-<company>-<rand>`).
    public_slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_item(overrides: &serde_json::Value) -> serde_json::Value {
        let mut item = serde_json::json!({
            "public_slug": "ml-engineer-acme-8f3k2",
            "title": "ML Engineer",
            "company": "Acme Robotics",
            "location": "Remote",
            "url": "https://freehire.me/jobs/ml-engineer-acme-8f3k2",
            "description": "<p>Build <strong>LLM</strong> apps &amp; ship them.</p>",
            "posted_at": "2026-08-30T01:49:31Z",
            "closed_at": null,
        });
        for (k, v) in overrides.as_object().unwrap() {
            item[k] = v.clone();
        }
        item
    }

    #[tokio::test]
    async fn parses_search_response_and_strips_html() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent/jobs/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [sample_item(&serde_json::json!({}))],
                "meta": {"count": 1, "page": 1, "total": 1},
            })))
            .mount(&server)
            .await;

        let src = FreehireSource::new()
            .with_base_url(server.uri())
            .with_query("machine learning");
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "ml-engineer-acme-8f3k2");
        assert_eq!(listings[0].company, "Acme Robotics");
        assert_eq!(listings[0].description, "Build LLM apps & ship them.");
    }

    #[tokio::test]
    async fn skips_closed_postings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent/jobs/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    sample_item(&serde_json::json!({})),
                    sample_item(&serde_json::json!({
                        "public_slug": "closed-role-xyz-1a2b3",
                        "title": "Closed Role",
                        "closed_at": "2026-08-01T00:00:00Z",
                    })),
                ],
            })))
            .mount(&server)
            .await;

        let src = FreehireSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "ml-engineer-acme-8f3k2");
    }

    #[tokio::test]
    async fn drops_rows_without_title() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent/jobs/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    sample_item(&serde_json::json!({})),
                    sample_item(&serde_json::json!({"public_slug": "no-title-1", "title": ""})),
                ],
            })))
            .mount(&server)
            .await;

        let src = FreehireSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
    }

    #[tokio::test]
    async fn sends_query_facet_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent/jobs/search"))
            .and(query_param("q", "rust robotics"))
            .and(query_param("limit", "10"))
            .and(query_param("remote", "remote"))
            .and(query_param("region", "eu"))
            .and(query_param("jobage", "14"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [sample_item(&serde_json::json!({}))],
            })))
            .mount(&server)
            .await;

        let src = FreehireSource::new()
            .with_base_url(server.uri())
            .with_query("rust robotics")
            .with_limit(10)
            .with_remote_only(true)
            .with_region("eu")
            .with_jobage(14);
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
    }

    #[tokio::test]
    async fn builds_freehire_url_when_item_url_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent/jobs/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [sample_item(&serde_json::json!({"url": null}))],
            })))
            .mount(&server)
            .await;

        let src = FreehireSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(
            listings[0].url,
            "https://freehire.me/jobs/ml-engineer-acme-8f3k2"
        );
    }

    #[tokio::test]
    async fn surfaces_http_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent/jobs/search"))
            .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
            .mount(&server)
            .await;

        let src = FreehireSource::new().with_base_url(server.uri());
        let err = src.discover().await.unwrap_err();
        assert!(
            matches!(err, SourceError::HttpStatus { status: 503, .. }),
            "got {err:?}"
        );
    }

    /// Live smoke test against the real freehire.me API — run manually
    /// with `cargo test -p careerai-sources --lib freehire::tests::live_smoke -- --ignored`.
    /// Confirms the endpoint contract (verified 2026-08-30):
    /// `GET /api/v1/agent/jobs/search` → `{"data": [ ... ]}` with
    /// `public_slug`/`title`/`company`/`url`/`description` fields.
    #[tokio::test]
    #[ignore = "live freehire.me smoke test — run manually"]
    async fn live_smoke() {
        let src = FreehireSource::new().with_query("ml").with_limit(3);
        let listings = src.discover().await.expect("live search");
        assert!(!listings.is_empty(), "live API returned no listings");
        for l in &listings {
            assert!(!l.title.is_empty());
            assert!(l.url.starts_with("http"));
        }
    }
}
