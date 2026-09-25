//! Email finding: guess common role-address patterns against a company
//! domain, verify the domain resolves via DNS, and optionally scrape the
//! company's /careers and /contact pages for contact addresses. Results
//! carry confidence scores in the 0.7–0.9 band (scraped > guessed,
//! MX-verified > resolving-only).
//!
//! DNS is abstracted behind [`DnsResolver`][crate::email_validator::DnsResolver]
//! so finding can be exercised offline with a mock; HTTP scraping is tested
//! against a local `wiremock` server.

use regex::Regex;

use crate::email_validator::DnsResolver;

/// Role-address local parts guessed for hiring contacts, in order of
/// likelihood.
const ROLE_PREFIXES: &[&str] = &["jobs", "hiring", "careers", "talent", "recruiting"];

/// Confidence for a guessed address whose domain resolves but has no MX.
const CONF_GUESS_RESOLVES: f64 = 0.7;
/// Confidence for a guessed address whose domain resolves AND has an MX.
const CONF_GUESS_MX: f64 = 0.8;
/// Confidence for an address scraped from a company page (careers/contact/job).
const CONF_SCRAPED: f64 = 0.9;

/// A discovered contact email with its provenance and a confidence score
/// in the 0.7–0.9 band (higher = more trustworthy).
#[derive(Debug, Clone, PartialEq)]
pub struct FoundEmail {
    pub email: String,
    pub source: String,
    pub confidence: f64,
}

/// Finds likely recruiter/hiring contact emails for a company by guessing
/// role-address patterns and scraping public careers/contact pages.
#[derive(Debug)]
pub struct EmailFinder<R: DnsResolver> {
    http: reqwest::Client,
    resolver: R,
    email_re: Regex,
}

impl<R: DnsResolver> EmailFinder<R> {
    /// Construct a finder over the given DNS resolver. Returns an error
    /// only if the HTTP client or extraction regex fails to build.
    pub fn new(resolver: R) -> std::result::Result<Self, String> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(3))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let email_re = Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}")
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            resolver,
            email_re,
        })
    }

    /// Find candidate contact emails for `company_domain`.
    ///
    /// `company_domain` may be a bare host (`acme.com`) or a full URL
    /// (`https://acme.com`); a bare host is scraped over `https://`. When
    /// `job_url` is provided it is also scraped for embedded addresses.
    pub async fn find_emails(
        &self,
        company_domain: &str,
        job_url: Option<&str>,
    ) -> Vec<FoundEmail> {
        let mut results = Vec::new();
        let bare = bare_host(company_domain);

        // Stage 1: guess role addresses if the domain resolves.
        let resolves = self
            .resolver
            .lookup_ip(&bare)
            .await
            .is_ok_and(|ips| !ips.is_empty());

        if resolves {
            let has_mx = self
                .resolver
                .lookup_mx(&bare)
                .await
                .is_ok_and(|mx| !mx.is_empty());
            let confidence = if has_mx {
                CONF_GUESS_MX
            } else {
                CONF_GUESS_RESOLVES
            };
            for prefix in ROLE_PREFIXES {
                results.push(FoundEmail {
                    email: format!("{prefix}@{bare}"),
                    source: "guess".to_owned(),
                    confidence,
                });
            }
        }

        // Stage 2: scrape /careers and /contact pages.
        let scrape_base = company_domain.trim_end_matches('/');
        for page in &["careers", "contact"] {
            let url = format!("{scrape_base}/{page}");
            scrape_page(&self.http, &self.email_re, &url, &mut results).await;
        }

        // Stage 3: optionally scrape the job posting.
        if let Some(job_url) = job_url {
            scrape_page(&self.http, &self.email_re, job_url, &mut results).await;
        }

        results
    }
}

/// Fetch `url`, extract email addresses from the body, and push any new
/// ones into `results` at `CONF_SCRAPED` confidence. Non-success responses
/// are silently skipped.
async fn scrape_page(
    http: &reqwest::Client,
    email_re: &Regex,
    url: &str,
    results: &mut Vec<FoundEmail>,
) {
    let Ok(resp) = http.get(url).send().await else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(body) = resp.text().await else {
        return;
    };
    for cap in email_re.find_iter(&body) {
        let email = cap.as_str().to_owned();
        if !results.iter().any(|e| e.email == email) {
            results.push(FoundEmail {
                email,
                source: "scrape".to_owned(),
                confidence: CONF_SCRAPED,
            });
        }
    }
}

/// Strip scheme and port from a host/URL to get the bare domain.
fn bare_host(uri: &str) -> String {
    let stripped = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))
        .unwrap_or(uri);
    stripped
        .rsplit_once(':')
        .map_or(stripped, |(host, _)| host)
        .to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Mock DNS resolver returning canned MX/IP results keyed by host.
    #[derive(Debug, Default, Clone)]
    struct MockResolver {
        mx: HashMap<String, Vec<String>>,
        ips: HashMap<String, Vec<IpAddr>>,
    }

    impl MockResolver {
        fn resolving(host: &str) -> Self {
            let mut r = Self::default();
            r.ips
                .insert(host.to_owned(), vec![IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))]);
            r
        }
        fn with_mx(mut self, host: &str, mx: &[&str]) -> Self {
            self.mx.insert(
                host.to_owned(),
                mx.iter().map(|s| (*s).to_owned()).collect(),
            );
            self
        }
    }

    #[async_trait::async_trait]
    impl DnsResolver for MockResolver {
        async fn lookup_mx(&self, domain: &str) -> Result<Vec<String>, String> {
            Ok(self.mx.get(domain).cloned().unwrap_or_default())
        }
        async fn lookup_ip(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            Ok(self.ips.get(host).cloned().unwrap_or_default())
        }
    }

    fn finder(resolver: MockResolver) -> EmailFinder<MockResolver> {
        EmailFinder::new(resolver).unwrap()
    }

    #[tokio::test]
    async fn guesses_role_emails_when_domain_resolves_with_mx() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let resolver = MockResolver::resolving(&bare).with_mx(&bare, &["mail.example.com."]);
        let f = finder(resolver);
        let found = f.find_emails(&host, None).await;

        for prefix in ROLE_PREFIXES {
            let email = format!("{prefix}@{bare}");
            assert!(
                found
                    .iter()
                    .any(|e| e.email == email && (e.confidence - CONF_GUESS_MX).abs() < 1e-9),
                "expected guessed {email} at confidence {CONF_GUESS_MX}, got {found:?}"
            );
        }
    }

    #[tokio::test]
    async fn guesses_use_lower_confidence_without_mx() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let resolver = MockResolver::resolving(&bare);
        let f = finder(resolver);
        let found = f.find_emails(&host, None).await;

        let jobs = found
            .iter()
            .find(|e| e.email == format!("jobs@{bare}"))
            .unwrap_or_else(|| panic!("jobs@{bare} missing: {found:?}"));
        assert!(
            (jobs.confidence - CONF_GUESS_RESOLVES).abs() < 1e-9,
            "no-MX guess should be {CONF_GUESS_RESOLVES}, got {}",
            jobs.confidence
        );
    }

    #[tokio::test]
    async fn skips_guesses_when_domain_does_not_resolve() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<p>email careers@acme.com</p>"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let resolver = MockResolver::default();
        let f = finder(resolver);
        let found = f.find_emails(&host, None).await;

        assert!(
            !found.iter().any(|e| e.email == format!("jobs@{bare}")),
            "guessed emails must be skipped when the domain does not resolve: {found:?}"
        );
        assert!(
            found.iter().any(|e| e.email == "careers@acme.com"),
            "scraped email should still be present: {found:?}"
        );
    }

    #[tokio::test]
    async fn scrapes_emails_from_careers_and_contact_pages() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html>Contact: careers@acme.com</html>"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<a href='mailto:contact@acme.com'>x</a>"),
            )
            .mount(&server)
            .await;

        let resolver = MockResolver::resolving(&bare).with_mx(&bare, &["mail.acme.com."]);
        let f = finder(resolver);
        let found = f.find_emails(&host, None).await;

        let careers = found
            .iter()
            .find(|e| e.email == "careers@acme.com")
            .unwrap_or_else(|| panic!("careers@acme.com missing: {found:?}"));
        assert!(
            (careers.confidence - CONF_SCRAPED).abs() < 1e-9,
            "scraped email should be {CONF_SCRAPED}, got {}",
            careers.confidence
        );
        assert!(
            found.iter().any(
                |e| e.email == "contact@acme.com" && (e.confidence - CONF_SCRAPED).abs() < 1e-9
            ),
            "contact@acme.com missing at scraped confidence: {found:?}"
        );
    }

    #[tokio::test]
    async fn scrapes_job_url_when_provided() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/jobs/42"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("Apply via hiring@acme.com today"),
            )
            .mount(&server)
            .await;

        let resolver = MockResolver::resolving(&bare);
        let f = finder(resolver);
        let job_url = format!("{host}/jobs/42");
        let found = f.find_emails(&host, Some(&job_url)).await;

        assert!(
            found
                .iter()
                .any(|e| e.email == "hiring@acme.com" && (e.confidence - CONF_SCRAPED).abs() < 1e-9),
            "hiring@acme.com scraped from job_url missing: {found:?}"
        );
    }

    #[tokio::test]
    async fn dedups_repeated_scraped_emails() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        let same = "careers@acme.com";
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("<p>{same}</p>")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("<p>{same}</p>")))
            .mount(&server)
            .await;

        let resolver = MockResolver::resolving(&bare).with_mx(&bare, &["mail."]);
        let f = finder(resolver);
        let found = f.find_emails(&host, None).await;

        let matching: Vec<_> = found.iter().filter(|e| e.email == same).collect();
        assert_eq!(
            matching.len(),
            1,
            "duplicate scraped emails must collapse: {found:?}"
        );
    }

    #[tokio::test]
    async fn ignores_non_success_status_when_scraping() {
        let server = MockServer::start().await;
        let host = server.uri();
        let bare = bare_host(&host);
        Mock::given(method("GET"))
            .and(path("/careers"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/contact"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let resolver = MockResolver::resolving(&bare).with_mx(&bare, &["mail."]);
        let f = finder(resolver);
        let found = f.find_emails(&host, None).await;

        assert!(
            found.iter().all(|e| e.source == "guess"),
            "non-success pages must yield no scraped emails: {found:?}"
        );
    }
}
