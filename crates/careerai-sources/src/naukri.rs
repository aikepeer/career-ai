//! Naukri.com India job search.
//!
//! Endpoint: `GET {base}/jobapi/v3/search?keywords=<kw>&location=<loc>&...`
//!
//! This is Naukri's undocumented internal SPA endpoint. Headers
//! `AppId: 109` / `SystemId: 109` are required — Naukri's public web client
//! sends them on every request and the API rejects callers that don't.
//! They are **not** secrets; both values are visible in the browser's
//! network panel on any naukri.com page load.
//!
//! Off by default in `SourcesConfig::NaukriSourceConfig::default`. User
//! opts in by flipping `sources.naukri.enabled = true` in
//! `config/local.yaml`.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://www.naukri.com";
/// Prefix used to absolutize `jdURL`, which upstream returns as a
/// site-relative path (e.g. `/job-listings-...`).
const NAUKRI_WEB_ORIGIN: &str = "https://www.naukri.com";
const DEFAULT_MAX_RESULTS: usize = 20;

#[derive(Debug)]
pub struct NaukriSource {
    base_url: String,
    http: Client,
    keywords: Vec<String>,
    location: String,
    max_results: usize,
}

impl Default for NaukriSource {
    fn default() -> Self {
        // Naukri rejects requests without AppId/SystemId/UA. A header-less
        // fallback is worse than a hard failure — `Client::builder().build()`
        // with only static default headers is infallible in practice; if it
        // does fail, reqwest's TLS/runtime init is broken and nothing else
        // in the binary will work either. Mirrors `remoteok.rs`.
        #[allow(clippy::expect_used)]
        let http = {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("AppId", reqwest::header::HeaderValue::from_static("109"));
            headers.insert("SystemId", reqwest::header::HeaderValue::from_static("109"));
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            Client::builder()
                .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
                .default_headers(headers)
                .build()
                .expect("build reqwest client with static Naukri headers")
        };
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
            keywords: vec!["machine learning".to_string()],
            location: "Delhi / NCR".to_string(),
            max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

impl NaukriSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the base URL (tests point at wiremock).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    #[must_use]
    pub fn with_keywords(mut self, keywords: Vec<String>) -> Self {
        self.keywords = keywords;
        self
    }

    #[must_use]
    pub fn with_location(mut self, location: String) -> Self {
        self.location = location;
        self
    }

    #[must_use]
    pub fn with_max_results(mut self, n: usize) -> Self {
        self.max_results = n;
        self
    }
}

#[async_trait]
impl Source for NaukriSource {
    fn name(&self) -> &'static str {
        "naukri"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/jobapi/v3/search");
        // Naukri's `keywords=` accepts a comma-separated value. Pass through
        // `.query(...)` so reqwest handles URL-encoding — never string-
        // interpolate user input into the URL (regression guard for the
        // Remotive fix on PR #2).
        let kw_joined = self.keywords.join(",");
        let max_results_str = self.max_results.to_string();
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("keywords", kw_joined.as_str()),
                ("location", self.location.as_str()),
                ("noOfResults", max_results_str.as_str()),
                ("src", "jobsearchDesk"),
                ("sid", ""),
                ("pageNo", "1"),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body: Payload = resp.json().await?;
        Ok(body
            .job_details
            .into_iter()
            .map(|j| {
                let location = j
                    .placeholders
                    .iter()
                    .find(|p| p.kind.as_deref() == Some("location"))
                    .and_then(|p| p.label.clone());
                let abs_url = if j.jd_url.starts_with("http") {
                    j.jd_url.clone()
                } else {
                    format!("{NAUKRI_WEB_ORIGIN}{}", j.jd_url)
                };
                let raw_json = serde_json::to_string(&j).ok();
                RawListing {
                    source: "naukri".to_string(),
                    external_id: j.job_id.unwrap_or_default(),
                    title: j.title.unwrap_or_default(),
                    company: j.company_name.unwrap_or_default(),
                    location,
                    url: abs_url,
                    description: html_to_text(&j.job_description.unwrap_or_default()),
                    raw_json,
                }
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct Payload {
    #[serde(rename = "jobDetails", default)]
    job_details: Vec<JobDetail>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct JobDetail {
    #[serde(rename = "jobId", default)]
    job_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "companyName", default)]
    company_name: Option<String>,
    #[serde(default)]
    placeholders: Vec<Placeholder>,
    #[serde(rename = "jdURL", default)]
    jd_url: String,
    #[serde(rename = "jobDescription", default)]
    job_description: Option<String>,
    #[serde(rename = "tagsAndSkills", default)]
    #[allow(dead_code)]
    tags_and_skills: Option<String>,
    #[serde(rename = "createdDate", default)]
    #[allow(dead_code)]
    created_date: Option<i64>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct Placeholder {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, header_regex, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_body() -> serde_json::Value {
        serde_json::json!({
            "jobDetails": [
                {
                    "jobId": "280125500001",
                    "title": "Senior ML Engineer",
                    "companyName": "Acme India",
                    "placeholders": [
                        {"type": "experience", "label": "5-10 Yrs"},
                        {"type": "salary", "label": "Not disclosed"},
                        {"type": "location", "label": "Bangalore, Delhi / NCR"}
                    ],
                    "jdURL": "/job-listings-senior-ml-engineer-acme-india-280125500001",
                    "jobDescription": "Build LLM systems on Kubernetes.",
                    "tagsAndSkills": "Python,ML,LLM,Kubernetes",
                    "createdDate": 1_712_345_678_000_i64
                },
                {
                    "jobId": "280125500002",
                    "title": "Robotics Perception Engineer",
                    "companyName": "Beta Robotics",
                    "placeholders": [
                        {"type": "location", "label": "Delhi / NCR"}
                    ],
                    "jdURL": "/job-listings-robotics-perception-engineer-beta-robotics-280125500002",
                    "jobDescription": "ROS2, SLAM, perception.",
                    "tagsAndSkills": "ROS2,SLAM",
                    "createdDate": 1_712_345_679_000_i64
                }
            ],
            "totalJobsDisplayed": 2
        })
    }

    #[tokio::test]
    async fn discovers_listings_against_mock() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jobapi/v3/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_body()))
            .mount(&server)
            .await;

        let src = NaukriSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 2);

        let first = &listings[0];
        assert_eq!(first.source, "naukri");
        assert_eq!(first.external_id, "280125500001");
        assert_eq!(first.title, "Senior ML Engineer");
        assert_eq!(first.company, "Acme India");
        assert_eq!(first.location.as_deref(), Some("Bangalore, Delhi / NCR"));
        assert_eq!(
            first.url,
            "https://www.naukri.com/job-listings-senior-ml-engineer-acme-india-280125500001"
        );

        let second = &listings[1];
        assert_eq!(second.external_id, "280125500002");
        assert_eq!(second.location.as_deref(), Some("Delhi / NCR"));
    }

    #[tokio::test]
    async fn strips_html_from_job_description() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "jobDetails": [{
                "jobId": "1",
                "title": "X",
                "companyName": "Y",
                "placeholders": [],
                "jdURL": "/x",
                "jobDescription": "<p>Build <strong>LLM</strong> apps.</p>"
            }],
            "totalJobsDisplayed": 1
        });
        Mock::given(method("GET"))
            .and(path("/jobapi/v3/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let src = NaukriSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].description, "Build LLM apps.");
    }

    #[tokio::test]
    async fn sends_required_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jobapi/v3/search"))
            .and(header("appid", "109"))
            .and(header("systemid", "109"))
            .and(header_regex("user-agent", "careerai"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_body()))
            .mount(&server)
            .await;

        let src = NaukriSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 2);
    }

    #[tokio::test]
    async fn http_error_surfaces_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jobapi/v3/search"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let src = NaukriSource::new().with_base_url(server.uri());
        let err = src.discover().await.unwrap_err();
        assert!(matches!(err, SourceError::HttpStatus { status: 403, .. }));
    }

    #[tokio::test]
    async fn passes_keywords_and_location_as_query_params() {
        // Regression test: if someone interpolates user input into the URL
        // path instead of using `.query(...)`, this test fires because
        // wiremock's `query_param` matcher only matches URL-encoded params.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jobapi/v3/search"))
            .and(query_param("keywords", "machine learning,llm"))
            .and(query_param("location", "Delhi / NCR"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_body()))
            .mount(&server)
            .await;

        let src = NaukriSource::new()
            .with_base_url(server.uri())
            .with_keywords(vec!["machine learning".to_string(), "llm".to_string()])
            .with_location("Delhi / NCR".to_string());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 2);
    }
}
