//! Indeed public RSS feed adapter.
//!
//! Endpoint: `GET https://rss.indeed.com/rss?q=<keywords>[&l=<loc>][&fromage=<days>]`
//!
//! Indeed publishes a stable, public, ToS-clean RSS feed for any search
//! query. No authentication, no aggressive rate limits. The feed is
//! standard RSS 2.0 with `<item>` entries; we map the entries to
//! [`RawListing`]s.
//!
//! Field mapping (item → listing):
//!
//! | listing field   | item source                                            |
//! |-----------------|--------------------------------------------------------|
//! | `title`         | `<title>` text content                                 |
//! | `company`       | derived from `<title>` (Indeed embeds company name)    |
//! | `url`           | `<link>` text content                                  |
//! | `description`   | `<description>` (HTML stripped via `util::html_to_text`) |
//! | `external_id`   | `<guid>` text content, or SHA truncation when missing  |
//! | `location`      | derived from `<title>` (best-effort `" - <Loc>"` tail) |
//!
//! Malformed `<item>` entries (missing `<link>`, empty `<title>`) are
//! skipped with a `warn!` event rather than aborting the discover()
//! call — the operator might still get useful listings from the rest
//! of the feed.

use std::num::NonZeroU32;
use std::sync::Arc;

use async_trait::async_trait;
use careerai_core::config::IndeedRssSourceConfig;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://rss.indeed.com";
const SOURCE_NAME: &str = "indeed_rss";

/// Per-instance read-side rate limiter. Same shape as `mcp_jobs.rs` —
/// see that module's comment for why we don't reuse the submit-side
/// limiter here.
type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

#[derive(Debug)]
pub struct IndeedRssSource {
    cfg: IndeedRssSourceConfig,
    base_url: String,
    http: Client,
    rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl IndeedRssSource {
    #[must_use]
    pub fn new(cfg: IndeedRssSourceConfig) -> Self {
        let rate_limiter = NonZeroU32::new(cfg.rate_per_minute)
            .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
        Self {
            cfg,
            base_url: DEFAULT_BASE_URL.to_string(),
            http: Client::new(),
            rate_limiter,
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for IndeedRssSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        if let Some(rl) = &self.rate_limiter {
            // Read-side permit. `until_ready` returns when the bucket
            // refills, bounded by `1 / rate_per_minute`.
            rl.until_ready().await;
        }

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/rss");
        let mut req = self
            .http
            .get(&url)
            .query(&[("q", self.cfg.keywords.as_str())]);
        if let Some(loc) = self.cfg.location.as_deref().filter(|s| !s.is_empty()) {
            req = req.query(&[("l", loc)]);
        }
        if let Some(days) = self.cfg.fromage {
            req = req.query(&[("fromage", days.to_string())]);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body = resp.text().await?;
        Ok(parse_feed(&body))
    }
}

/// Internal mutable item builder used while we walk the XML stream.
///
/// We deliberately do not derive `serde::Deserialize` on a struct: the
/// `description` element contains HTML (often with embedded `<br>` and
/// other tags) and `quick-xml`'s `serialize` feature does not strip it
/// for us. Walking the events stream lets us hand the description to
/// `util::html_to_text` directly.
#[derive(Debug, Default)]
struct ItemBuilder {
    title: String,
    link: String,
    description: String,
    guid: String,
    pub_date: String,
}

/// Parse an RSS 2.0 feed, returning every successfully-mapped listing.
/// Malformed `<item>` entries are skipped with a `warn!` log; a single
/// bad entry never aborts the whole feed.
fn parse_feed(xml: &str) -> Vec<RawListing> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut listings = Vec::new();
    let mut buf = Vec::new();
    let mut in_item = false;
    let mut current = ItemBuilder::default();
    let mut current_field: Option<&'static str> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).unwrap_or("");
                match tag {
                    "item" => {
                        in_item = true;
                        current = ItemBuilder::default();
                    }
                    "title" if in_item => current_field = Some("title"),
                    "link" if in_item => current_field = Some("link"),
                    "description" if in_item => current_field = Some("description"),
                    "guid" if in_item => current_field = Some("guid"),
                    "pubDate" if in_item => current_field = Some("pubDate"),
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).unwrap_or("");
                if tag == "item" && in_item {
                    in_item = false;
                    if let Some(listing) = build_listing(&current) {
                        listings.push(listing);
                    } else {
                        warn!(
                            link = %current.link,
                            title = %current.title,
                            "indeed_rss: skipping malformed item",
                        );
                    }
                } else if matches!(tag, "title" | "link" | "description" | "guid" | "pubDate") {
                    current_field = None;
                }
            }
            Ok(Event::Text(t)) => {
                if !in_item {
                    continue;
                }
                let Some(field) = current_field else { continue };
                let raw = match t.xml_content() {
                    Ok(s) => s.into_owned(),
                    Err(e) => {
                        warn!(
                            field = %field,
                            error = %e,
                            "indeed_rss: failed to decode text; skipping fragment",
                        );
                        continue;
                    }
                };
                push_field(&mut current, field, &raw);
            }
            Ok(Event::CData(t)) => {
                if !in_item {
                    continue;
                }
                let Some(field) = current_field else { continue };
                let raw = match t.xml_content() {
                    Ok(s) => s.into_owned(),
                    Err(e) => {
                        warn!(
                            field = %field,
                            error = %e,
                            "indeed_rss: failed to decode cdata; skipping fragment",
                        );
                        continue;
                    }
                };
                push_field(&mut current, field, &raw);
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                warn!(error = %e, "indeed_rss: xml parse error; stopping");
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    listings
}

fn push_field(current: &mut ItemBuilder, field: &'static str, raw: &str) {
    match field {
        "title" => current.title.push_str(raw),
        "link" => current.link.push_str(raw),
        "description" => current.description.push_str(raw),
        "guid" => current.guid.push_str(raw),
        "pubDate" => current.pub_date.push_str(raw),
        _ => {}
    }
}

fn build_listing(item: &ItemBuilder) -> Option<RawListing> {
    if item.title.trim().is_empty() || item.link.trim().is_empty() {
        return None;
    }
    let (title, company, location) = split_title(&item.title);
    let external_id = if item.guid.trim().is_empty() {
        derive_external_id(&item.link)
    } else {
        item.guid.trim().to_owned()
    };
    let description = html_to_text(&item.description);
    Some(RawListing {
        source: SOURCE_NAME.to_owned(),
        external_id,
        title,
        company,
        location,
        url: item.link.trim().to_owned(),
        description,
        raw_json: None,
    })
}

/// Indeed RSS embeds the company and (sometimes) location in the
/// `<title>` field, separated by " - ". Common shapes:
///   * `"Senior Engineer - Acme Corp"`
///   * `"Senior Engineer - Acme Corp - Remote"`
///   * `"Senior Engineer - Acme Corp - San Francisco, CA"`
///   * `"Senior Engineer"` (rare; no separator)
///
/// We split on `" - "`. With one separator, the right half is the
/// company. With two, the right half is the location and the middle is
/// the company. Anything more is folded into the company field — we'd
/// rather over-attribute than guess.
fn split_title(raw: &str) -> (String, String, Option<String>) {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split(" - ").collect();
    match parts.as_slice() {
        [title] => ((*title).trim().to_owned(), String::new(), None),
        [title, company] => (
            (*title).trim().to_owned(),
            (*company).trim().to_owned(),
            None,
        ),
        [title, company, rest @ ..] => {
            let location = rest.join(" - ").trim().to_owned();
            let location = if location.is_empty() {
                None
            } else {
                Some(location)
            };
            (
                (*title).trim().to_owned(),
                (*company).trim().to_owned(),
                location,
            )
        }
        [] => (String::new(), String::new(), None),
    }
}

/// SHA256 over `(SOURCE_NAME, url)` truncated to 16 hex chars. Used
/// when the feed item omits `<guid>`.
fn derive_external_id(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_NAME.as_bytes());
    hasher.update(b"\0");
    hasher.update(url.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE_XML: &str = include_str!("../tests/fixtures/indeed_rss_sample.xml");

    #[test]
    fn split_title_handles_one_separator() {
        let (title, company, loc) = split_title("Senior Engineer - Acme Corp");
        assert_eq!(title, "Senior Engineer");
        assert_eq!(company, "Acme Corp");
        assert_eq!(loc, None);
    }

    #[test]
    fn split_title_handles_two_separators_with_location() {
        let (title, company, loc) = split_title("Senior Engineer - Acme Corp - San Francisco, CA");
        assert_eq!(title, "Senior Engineer");
        assert_eq!(company, "Acme Corp");
        assert_eq!(loc.as_deref(), Some("San Francisco, CA"));
    }

    #[test]
    fn split_title_handles_no_separator() {
        let (title, company, loc) = split_title("Senior Engineer");
        assert_eq!(title, "Senior Engineer");
        assert_eq!(company, "");
        assert_eq!(loc, None);
    }

    #[test]
    fn parse_feed_maps_two_listings_from_fixture() {
        let listings = parse_feed(SAMPLE_XML);
        assert_eq!(listings.len(), 2, "fixture has 2 valid items");

        // First item has a guid, the second falls back to derive_external_id.
        let first = &listings[0];
        assert_eq!(first.title, "Senior AI Engineer");
        assert_eq!(first.company, "Acme Robotics");
        assert_eq!(first.location.as_deref(), Some("Remote"));
        assert_eq!(first.url, "https://www.indeed.com/viewjob?jk=abc123");
        assert_eq!(first.external_id, "abc123-stable-guid");
        assert!(
            first.description.contains("Build LLM agents"),
            "description should be HTML-stripped, got: {}",
            first.description,
        );
        assert_eq!(first.source, "indeed_rss");

        let second = &listings[1];
        assert_eq!(second.title, "Robotics Software Engineer");
        assert_eq!(second.company, "Beta Labs");
        assert_eq!(second.location.as_deref(), Some("Bengaluru, KA"));
        // No <guid> in fixture for item 2 → derived hash.
        assert_eq!(second.external_id.len(), 16);
        assert!(second.external_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_feed_skips_malformed_items() {
        // Two items: first valid, second missing <link>. Only one mapped.
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>Indeed</title>
<item>
  <title>Good Job - Acme</title>
  <link>https://www.indeed.com/viewjob?jk=ok</link>
  <description><![CDATA[<p>fine</p>]]></description>
  <guid>ok-guid</guid>
</item>
<item>
  <title>Bad Job - NoLink Co</title>
  <description>missing link</description>
</item>
</channel></rss>"#;
        let listings = parse_feed(xml);
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "ok-guid");
    }

    #[test]
    fn parse_feed_tolerates_bad_dates_without_panic() {
        // pubDate is currently unused in mapping but we still parse it.
        // A garbage date must not abort the rest of the item.
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<item>
  <title>Eng - Acme</title>
  <link>https://www.indeed.com/viewjob?jk=x</link>
  <description>hi</description>
  <pubDate>not-a-real-date</pubDate>
  <guid>x</guid>
</item>
</channel></rss>"#;
        let listings = parse_feed(xml);
        assert_eq!(listings.len(), 1);
    }

    #[test]
    fn parse_feed_handles_empty_feed() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel></channel></rss>"#;
        let listings = parse_feed(xml);
        assert!(listings.is_empty());
    }

    #[tokio::test]
    async fn discover_hits_endpoint_with_query_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rss"))
            .and(query_param("q", "AI engineer remote"))
            .and(query_param("fromage", "7"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(SAMPLE_XML)
                    .insert_header("content-type", "application/rss+xml"),
            )
            .mount(&server)
            .await;

        let cfg = IndeedRssSourceConfig {
            enabled: true,
            keywords: "AI engineer remote".into(),
            location: None,
            fromage: Some(7),
            rate_per_minute: 0,
        };
        let src = IndeedRssSource::new(cfg).with_base_url(server.uri());
        let listings = src.discover().await.unwrap();
        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].source, "indeed_rss");
    }

    #[tokio::test]
    async fn discover_propagates_http_status_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rss"))
            .respond_with(ResponseTemplate::new(429).set_body_string("Too Many Requests"))
            .mount(&server)
            .await;

        let cfg = IndeedRssSourceConfig {
            enabled: true,
            keywords: "x".into(),
            ..Default::default()
        };
        let src = IndeedRssSource::new(cfg).with_base_url(server.uri());
        let err = src.discover().await.expect_err("429 should error");
        match err {
            SourceError::HttpStatus { status, .. } => assert_eq!(status, 429),
            other => panic!("expected HttpStatus, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_listing_shape_from_fixture() {
        let listings = parse_feed(SAMPLE_XML);
        insta::assert_yaml_snapshot!("indeed_rss_listings", listings);
    }
}
