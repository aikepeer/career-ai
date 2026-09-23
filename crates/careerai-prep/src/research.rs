//! Allowlisted, read-only company research for interview prep.

use std::collections::BTreeSet;
use std::time::Duration;

use reqwest::Url;

use crate::error::{PrepError, Result};

/// Fetch approved HTTPS pages. Employer URLs are allowed automatically;
/// news URLs must belong to an explicitly supplied domain allowlist.
pub async fn fetch_approved_sources(
    urls: &[String],
    employer_url: &str,
    selected_domains: &[String],
) -> Result<Vec<String>> {
    let approved = validate_urls(urls, employer_url, selected_domains)?;
    if approved.is_empty() {
        return Ok(Vec::new());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("career-ai-prep/0.1")
        .build()
        .map_err(|err| PrepError::Backend(format!("research client: {err}")))?;
    let mut excerpts = Vec::with_capacity(approved.len());
    for url in approved {
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|err| PrepError::Backend(format!("research GET {url}: {err}")))?
            .error_for_status()
            .map_err(|err| PrepError::Backend(format!("research GET {url}: {err}")))?;
        let body = response
            .text()
            .await
            .map_err(|err| PrepError::Backend(format!("research body {url}: {err}")))?;
        let excerpt: String = body.chars().take(12_000).collect();
        excerpts.push(format!("{url}\n{excerpt}"));
    }
    Ok(excerpts)
}

fn validate_urls(
    urls: &[String],
    employer_url: &str,
    selected_domains: &[String],
) -> Result<Vec<Url>> {
    let employer = Url::parse(employer_url)
        .map_err(|err| PrepError::Backend(format!("invalid employer URL: {err}")))?;
    let employer_host = employer
        .host_str()
        .ok_or_else(|| PrepError::Backend("employer URL has no host".into()))?;
    let domains: BTreeSet<String> = selected_domains
        .iter()
        .map(|domain| {
            domain
                .trim()
                .trim_start_matches("www.")
                .to_ascii_lowercase()
        })
        .filter(|domain| !domain.is_empty())
        .collect();
    let mut approved = Vec::new();
    for raw in urls {
        let url = Url::parse(raw)
            .map_err(|err| PrepError::Backend(format!("invalid research URL: {err}")))?;
        if url.scheme() != "https" {
            return Err(PrepError::Backend(format!(
                "research URL must use https: {url}"
            )));
        }
        let host = url
            .host_str()
            .ok_or_else(|| PrepError::Backend(format!("research URL has no host: {url}")))?
            .trim_start_matches("www.")
            .to_ascii_lowercase();
        let employer_match = host == employer_host || host.ends_with(&format!(".{employer_host}"));
        let selected_match = domains
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
        if !employer_match && !selected_match {
            return Err(PrepError::Backend(format!(
                "research host is not approved: {host}"
            )));
        }
        approved.push(url);
    }
    Ok(approved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn employer_and_selected_news_domains_are_allowed() {
        let urls = vec![
            "https://jobs.acme.example/news".into(),
            "https://news.example.org/story".into(),
        ];
        let approved = validate_urls(
            &urls,
            "https://acme.example/jobs/1",
            &["example.org".into()],
        )
        .unwrap();
        assert_eq!(approved.len(), 2);
    }

    #[test]
    fn unapproved_domains_and_http_are_rejected() {
        let error = validate_urls(
            &["https://unapproved.example/story".into()],
            "https://acme.example/jobs/1",
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("not approved"));
    }
}
