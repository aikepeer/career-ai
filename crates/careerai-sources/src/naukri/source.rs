//! Naukri source driver: HTTP client, discovery, URL absolutization.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

use super::types::{Payload, DEFAULT_BASE_URL, DEFAULT_MAX_RESULTS, NAUKRI_WEB_ORIGIN};

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
            .timeout(Duration::from_secs(30))
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
pub(crate) fn absolutize_jd_url(jd_url: &str) -> Option<String> {
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
