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
            .filter_map(|j| {
                // Skip rows with no job_id — `insert_or_ignore` dedupes
                // on (source, external_id), so all empty-id rows would
                // collapse into one and silently drop later listings.
                let external_id = j.job_id.clone().filter(|s| !s.is_empty())?;
                let location = j
                    .placeholders
                    .iter()
                    .find(|p| p.kind.as_deref() == Some("location"))
                    .and_then(|p| p.label.clone());
                let abs_url = absolutize_jd_url(&j.jd_url)?;
                let raw_json = serde_json::to_string(&j).ok();
                Some(RawListing {
                    source: "naukri".to_string(),
                    external_id,
                    title: j.title.unwrap_or_default(),
                    company: j.company_name.unwrap_or_default(),
                    location,
                    url: abs_url,
                    description: html_to_text(&j.job_description.unwrap_or_default()),
                    raw_json,
                })
            })
            .collect())
    }
}

/// Absolutize a Naukri-supplied `jdURL`, refusing any value that isn't
/// rooted at `https://www.naukri.com`. An attacker-controlled feed
/// could otherwise return an absolute URL pointing anywhere — that URL
/// then lands in `listings.url` and downstream submitters / templates.
///
/// Returns `None` for empty or off-origin URLs (caller treats as
/// "skip this row").
fn absolutize_jd_url(jd_url: &str) -> Option<String> {
    let trimmed = jd_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Protocol-relative `//foo.com/bar` resolves to whatever scheme the
    // page is served over — unsafe in any context. Reject.
    if trimmed.starts_with("//") {
        return None;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        if trimmed.starts_with("https://www.naukri.com/")
            || trimmed.starts_with("http://www.naukri.com/")
        {
            return Some(trimmed.to_string());
        }
        return None;
    }
    // Site-relative path: must start with `/` and not collapse the
    // origin (`/foo` is OK; `foo` without a leading `/` is suspicious).
    if !trimmed.starts_with('/') {
        return None;
    }
    Some(format!("{NAUKRI_WEB_ORIGIN}{trimmed}"))
}

#[derive(Debug, Deserialize)]
struct Payload {
    #[serde(rename = "jobDetails", default)]
    job_details: Vec<JobDetail>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct JobDetail {
    /// Naukri's payload has historically returned `jobId` as both a
    /// string ("280125500001") and a JSON number (280125500001) at
    /// different points in the API lifecycle. Accept either; the
    /// deserializer normalizes to `Option<String>`.
    #[serde(
        rename = "jobId",
        default,
        deserialize_with = "deserialize_string_or_number"
    )]
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

/// Accept JSON `"280125"`, `280125`, or `null` for fields that should
/// land as `Option<String>`. Naukri's API has shipped jobId in both
/// string and integer forms across versions; tolerate both rather than
/// drop a whole page of listings on a serde error.
fn deserialize_string_or_number<'de, D>(de: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<String>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("string, integer, or null")
        }
        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.to_owned()))
        }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        #[allow(clippy::cast_possible_truncation)]
        fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
            // Treat as integer-ish; reject NaN/Infinity. JobIds are
            // expected to be in the i64 range; the truncation is the
            // intended behavior for any value that arrived as a float.
            if v.is_finite() {
                Ok(Some((v as i64).to_string()))
            } else {
                Err(de::Error::custom("non-finite number for jobId"))
            }
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            de.deserialize_any(V)
        }
    }
    de.deserialize_any(V)
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

    #[test]
    fn absolutize_jd_url_rejects_off_origin_absolute() {
        // Site-relative path stays site-relative.
        assert_eq!(
            absolutize_jd_url("/foo-bar-1234"),
            Some("https://www.naukri.com/foo-bar-1234".to_owned())
        );
        // Same-origin absolute URL passes through.
        assert_eq!(
            absolutize_jd_url("https://www.naukri.com/foo-1234"),
            Some("https://www.naukri.com/foo-1234".to_owned())
        );
        // Off-origin absolute URL → rejected.
        assert_eq!(absolutize_jd_url("https://evil.com/phish"), None);
        // Protocol-relative → rejected.
        assert_eq!(absolutize_jd_url("//evil.com/foo"), None);
        // Empty / whitespace → rejected.
        assert_eq!(absolutize_jd_url(""), None);
        assert_eq!(absolutize_jd_url("   "), None);
        // Bare token (no leading slash) → rejected.
        assert_eq!(absolutize_jd_url("foo"), None);
    }

    #[tokio::test]
    async fn skips_listings_with_empty_or_missing_job_id() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "jobDetails": [
                {
                    "jobId": "real-1",
                    "title": "Engineer",
                    "companyName": "Co",
                    "placeholders": [],
                    "jdURL": "/jd/1",
                    "jobDescription": "x"
                },
                {
                    "jobId": "",
                    "title": "Empty ID",
                    "companyName": "Co",
                    "placeholders": [],
                    "jdURL": "/jd/2",
                    "jobDescription": "x"
                },
                {
                    // jobId omitted entirely
                    "title": "Missing ID",
                    "companyName": "Co",
                    "placeholders": [],
                    "jdURL": "/jd/3",
                    "jobDescription": "x"
                }
            ]
        });
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let src = NaukriSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        // Only the row with a real jobId survives.
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "real-1");
    }

    #[tokio::test]
    async fn accepts_numeric_job_id_in_response() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "jobDetails": [
                {
                    // Number, not string — must round-trip via the
                    // string-or-number deserializer.
                    "jobId": 280_125_500_001_i64,
                    "title": "ML Engineer",
                    "companyName": "Co",
                    "placeholders": [],
                    "jdURL": "/jd/1",
                    "jobDescription": "x"
                }
            ]
        });
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let src = NaukriSource::new().with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "280125500001");
    }
}
