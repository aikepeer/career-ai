//! Hacker News "Who is hiring?" via Algolia search API.
//!
//! Endpoint: `GET https://hn.algolia.com/api/v1/search`
//! Searches HN comments for "Ask HN: Who is hiring?" threads.
//! Each comment is a job posting.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::base::{RawListing, Source, SourceError};

const ALGOLIA_BASE: &str = "https://hn.algolia.com/api/v1/search";

/// Tags filter to search only comments (not stories).
const TAGS: &str = "comment";
/// Default search query — targets the monthly "Ask HN: Who is hiring?" threads.
const DEFAULT_QUERY: &str = "Ask HN: Who is hiring";

#[derive(Debug)]
pub struct HackerNewsSource {
    http: Client,
    base_url: String,
    query: String,
    max_hits: usize,
}

impl Default for HackerNewsSource {
    fn default() -> Self {
        #[allow(clippy::expect_used)]
        let http = Client::builder()
            .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
            .build()
            .expect("build reqwest client");
        Self {
            http,
            base_url: ALGOLIA_BASE.to_string(),
            query: DEFAULT_QUERY.to_string(),
            max_hits: 100,
        }
    }
}

impl HackerNewsSource {
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
    pub fn with_max_hits(mut self, max: usize) -> Self {
        self.max_hits = max;
        self
    }
}

#[async_trait]
impl Source for HackerNewsSource {
    fn name(&self) -> &'static str {
        "hackernews"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let resp = self
            .http
            .get(&self.base_url)
            .query(&[
                ("query", self.query.as_str()),
                ("tags", TAGS),
                ("hitsPerPage", &self.max_hits.to_string()),
            ])
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let envelope: AlgoliaResponse = resp.json().await?;
        Ok(envelope.hits.into_iter().filter_map(parse_hit).collect())
    }
}

fn parse_hit(hit: AlgoliaHit) -> Option<RawListing> {
    let text = hit.comment_text?;
    // HN "Who is hiring?" comments follow conventions:
    // First line is often "Company | Location | Remote/Onsite | Full-time"
    // or just a job title + description.
    let first_line = text.lines().next()?;
    let (company, title) = parse_first_line(first_line);

    let external_id = hit.object_id.clone().unwrap_or_else(|| {
        // Fall back to a hash of the first 80 chars if no object_id
        let prefix = &text[..text.len().min(80)];
        format!("{:x}", fxhash(prefix))
    });

    Some(RawListing {
        source: "hackernews".to_string(),
        external_id,
        title,
        company,
        location: extract_location(&text),
        url: hit.story_url.unwrap_or_else(|| {
            hit.object_id
                .as_ref()
                .map(|id| format!("https://news.ycombinator.com/item?id={id}"))
                .unwrap_or_default()
        }),
        description: text,
        raw_json: None,
    })
}

/// Parse the first line of an HN job comment.
/// Common formats:
///   "Company | Location | Remote | Full-time"
///   "Company: Job Title"
///   "Job Title at Company"
fn parse_first_line(line: &str) -> (String, String) {
    let trimmed = line.trim();
    if let Some((left, right)) = trimmed.split_once(" | ") {
        // "Company | Location | ..." → company is first field, title derived from rest
        let company = left.trim().to_string();
        // Try to extract a title from the second segment
        let title = right
            .split(" | ")
            .next()
            .unwrap_or(right)
            .trim()
            .to_string();
        let title = if title.is_empty() {
            trimmed.to_string()
        } else {
            title
        };
        (company, title)
    } else if let Some((company, title)) = trimmed.split_once(": ") {
        (company.trim().to_string(), title.trim().to_string())
    } else if let Some((title, company)) = trimmed.split_once(" at ") {
        (company.trim().to_string(), title.trim().to_string())
    } else {
        (String::new(), trimmed.to_string())
    }
}

/// Heuristic: look for "Remote", "Remote (", or city names in the text.
fn extract_location(text: &str) -> Option<String> {
    for line in text.lines().take(5) {
        let lower = line.to_lowercase();
        if lower.contains("remote") {
            if let Some(idx) = lower.find("remote") {
                let rest = &line[idx..];
                if let Some(end) = rest.find(')') {
                    return Some(rest[..=end].trim().to_string());
                }
                return Some("Remote".to_string());
            }
        }
    }
    None
}

/// Simple FNV-1a hash for generating stable IDs from text.
fn fxhash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in s.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Debug, Deserialize)]
struct AlgoliaResponse {
    #[serde(default)]
    hits: Vec<AlgoliaHit>,
}

#[derive(Debug, Deserialize)]
struct AlgoliaHit {
    #[serde(default, rename = "objectID")]
    object_id: Option<String>,
    #[serde(default, rename = "comment_text")]
    comment_text: Option<String>,
    #[serde(default, rename = "story_url")]
    story_url: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn parses_algolia_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hits": [
                    {
                        "objectID": "39587219",
                        "comment_text": "Acme Robotics | Berlin | Remote (EU) | Full-time\n\nLooking for an embedded systems engineer to work on sensor fusion and RTOS development. Rust experience is a plus.\n\nApply at careers@acme.com",
                        "story_url": "https://news.ycombinator.com/item?id=39585000"
                    },
                    {
                        "objectID": "39587220",
                        "comment_text": "AIStartup: ML Platform Engineer\n\nBuild infrastructure for training and serving LLMs at scale.\n\nhttps://aistartup.com/jobs",
                        "story_url": "https://news.ycombinator.com/item?id=39585000"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let source = HackerNewsSource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();

        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].external_id, "39587219");
        assert_eq!(listings[0].company, "Acme Robotics");
        assert_eq!(listings[0].location.as_deref(), Some("Remote (EU)"));
        assert!(listings[0].description.contains("sensor fusion"));
        assert_eq!(listings[1].company, "AIStartup");
        assert_eq!(listings[1].title, "ML Platform Engineer");
    }

    #[tokio::test]
    async fn skips_hits_without_comment_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hits": [
                    {"objectID": "1", "comment_text": "Company | NYC | Job Title\n\nDesc"},
                    {"objectID": "2", "comment_text": null}
                ]
            })))
            .mount(&server)
            .await;

        let source = HackerNewsSource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].company, "Company");
    }

    #[test]
    fn parses_pipe_delimited_first_line() {
        let (company, title) = parse_first_line("Acme Corp | San Francisco | Remote | Full-time");
        assert_eq!(company, "Acme Corp");
        assert_eq!(title, "San Francisco");
    }

    #[test]
    fn parses_colon_format() {
        let (company, title) = parse_first_line("StartupCo: Senior Engineer");
        assert_eq!(company, "StartupCo");
        assert_eq!(title, "Senior Engineer");
    }

    #[test]
    fn parses_at_format() {
        let (company, title) = parse_first_line("Engineer at BigCorp");
        assert_eq!(company, "BigCorp");
        assert_eq!(title, "Engineer");
    }

    #[test]
    fn extracts_remote_location() {
        let loc = extract_location("Company | Remote (EU) | Full-time\nMore text");
        assert_eq!(loc.as_deref(), Some("Remote (EU)"));
    }

    #[test]
    fn extracts_plain_remote() {
        let loc = extract_location("Company | Remote | Full-time\nMore text");
        assert_eq!(loc.as_deref(), Some("Remote"));
    }
}
