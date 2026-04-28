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
use std::time::Duration;

use async_trait::async_trait;
use careerai_core::config::IndeedRssSourceConfig;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://rss.indeed.com";
const SOURCE_NAME: &str = "indeed_rss";

/// Hard ceiling on `discover()` HTTP wall time. Indeed's edge can stall
/// indefinitely on 5xx upstream incidents; without a timeout one bad
/// tick will pin a scheduler job until the daemon is killed.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// User-agent header sent with every request. RemoteOK hard-rejects
/// UA-less requests; Indeed's behavior is undocumented but its CDN is
/// known to throttle anonymous traffic. Pinning the same UA the rest
/// of the workspace uses keeps logs greppable and traffic identifiable.
const USER_AGENT: &str = "careerai/0.1 (+https://github.com/justdoGIT/career-ai)";

/// Indeed's `fromage` parameter accepts an integer "posted within N
/// days" window. Empirically the upstream accepts 1..=30; values above
/// 30 are silently treated as 30, and `0` returns nothing useful. We
/// clamp at the adapter boundary so a typo in `local.yaml` produces a
/// usable feed instead of an empty one.
const FROMAGE_MIN: u32 = 1;
const FROMAGE_MAX: u32 = 30;

/// Per-instance read-side rate limiter. Same shape as `mcp_jobs.rs` —
/// see that module's comment for why we don't reuse the submit-side
/// limiter here.
type ReadRateLimiter = RateLimiter<
    NotKeyed,
    InMemoryState,
    DefaultClock,
    NoOpMiddleware<governor::clock::QuantaInstant>,
>;

/// Per-source-name interner for the read-side rate limiter, mirroring the
/// `mcp_jobs.rs` pattern (see lines 74–107 there for the rationale).
///
/// `careerai-pipeline::build_sources` reconstructs `IndeedRssSource` on
/// every cron tick. Without this cache, each tick rebuilds the limiter
/// with a full bucket and `rate_per_minute` is effectively unenforced in
/// daemon mode. Holding the `Arc<RateLimiter>` in a process-wide
/// `OnceLock<Mutex<HashMap>>` keyed by source name means the bucket state
/// survives reconstruction — the per-minute cap is honored across ticks.
///
/// Today there is exactly one Indeed RSS source (`SOURCE_NAME`), but the
/// keying matches `mcp_jobs.rs` so adding multi-instance support later is
/// a non-event.
type InternMap = std::sync::Mutex<std::collections::HashMap<String, Option<Arc<ReadRateLimiter>>>>;
static SOURCE_STATE: std::sync::OnceLock<InternMap> = std::sync::OnceLock::new();

fn intern_rate_limiter(name: &str, rate_per_minute: u32) -> Option<Arc<ReadRateLimiter>> {
    let map = SOURCE_STATE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // `lock()` only fails if a previous holder panicked. The cached state
    // is `Option<Arc<RateLimiter>>`; recovery is safe.
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = guard.get(name) {
        return entry.clone();
    }
    let rl = NonZeroU32::new(rate_per_minute)
        .map(|n| Arc::new(RateLimiter::direct(Quota::per_minute(n))));
    guard.insert(name.to_owned(), rl.clone());
    rl
}

#[derive(Debug)]
pub struct IndeedRssSource {
    cfg: IndeedRssSourceConfig,
    base_url: String,
    http: Client,
    /// Shared with all other `IndeedRssSource` instances constructed with
    /// the same source name (today: always `SOURCE_NAME`). The
    /// token-bucket state lives in the `Arc`, so a new construction on
    /// the next cron tick via `build_sources()` does not reset the
    /// bucket — the per-minute cap is honored across ticks.
    rate_limiter: Option<Arc<ReadRateLimiter>>,
}

impl IndeedRssSource {
    /// Build a source from its config block. The HTTP client carries a
    /// fixed `HTTP_TIMEOUT` and a stable `USER_AGENT`; if those static
    /// settings are unrepresentable, reqwest's TLS / runtime init is
    /// broken and nothing else in the binary will work either, so we
    /// panic visibly rather than degrade silently.
    #[must_use]
    pub fn new(cfg: IndeedRssSourceConfig) -> Self {
        let rate_limiter = intern_rate_limiter(SOURCE_NAME, cfg.rate_per_minute);
        #[allow(clippy::expect_used)]
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("build reqwest client with static UA + timeout");
        Self {
            cfg,
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
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
            let clamped = days.clamp(FROMAGE_MIN, FROMAGE_MAX);
            if clamped != days {
                warn!(
                    requested = days,
                    clamped,
                    "indeed_rss: fromage out of range; clamped to {FROMAGE_MIN}..={FROMAGE_MAX}",
                );
            }
            req = req.query(&[("fromage", clamped.to_string())]);
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
    // We deliberately keep `trim_text` OFF: `quick-xml` 0.38 emits XML
    // entity references (`&amp;`) as their own event, splitting the
    // surrounding text. With `trim_text(true)` the whitespace adjacent
    // to those splits is also stripped, so "Procter & Gamble" becomes
    // "Procter&Gamble" once we glue the fragments back together. Field
    // values are trimmed once at item-finalization in `build_listing`.
    reader.config_mut().trim_text(false);

    let mut listings = Vec::new();
    let mut buf = Vec::new();
    let mut in_item = false;
    let mut current = ItemBuilder::default();
    let mut current_field: Option<&'static str> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                handle_start(
                    e.name().as_ref(),
                    &mut in_item,
                    &mut current,
                    &mut current_field,
                );
            }
            Ok(Event::End(e)) => {
                handle_end(
                    e.name().as_ref(),
                    &mut in_item,
                    &current,
                    &mut current_field,
                    &mut listings,
                );
            }
            Ok(Event::Text(t)) => {
                if !in_item {
                    continue;
                }
                let Some(field) = current_field else { continue };
                match t.xml_content() {
                    Ok(s) => push_field(&mut current, field, &s),
                    Err(e) => warn!(
                        field = %field, error = %e,
                        "indeed_rss: failed to decode text; skipping fragment",
                    ),
                }
            }
            Ok(Event::CData(t)) => {
                if !in_item {
                    continue;
                }
                let Some(field) = current_field else { continue };
                match t.xml_content() {
                    Ok(s) => push_field(&mut current, field, &s),
                    Err(e) => warn!(
                        field = %field, error = %e,
                        "indeed_rss: failed to decode cdata; skipping fragment",
                    ),
                }
            }
            Ok(Event::GeneralRef(g)) => {
                if !in_item {
                    continue;
                }
                if let Some(field) = current_field {
                    handle_entity(g.as_ref(), field, &mut current);
                }
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

fn handle_start(
    name: &[u8],
    in_item: &mut bool,
    current: &mut ItemBuilder,
    current_field: &mut Option<&'static str>,
) {
    let tag = std::str::from_utf8(name).unwrap_or("");
    match tag {
        "item" => {
            *in_item = true;
            *current = ItemBuilder::default();
        }
        "title" if *in_item => *current_field = Some("title"),
        "link" if *in_item => *current_field = Some("link"),
        "description" if *in_item => *current_field = Some("description"),
        "guid" if *in_item => *current_field = Some("guid"),
        "pubDate" if *in_item => *current_field = Some("pubDate"),
        _ => {}
    }
}

fn handle_end(
    name: &[u8],
    in_item: &mut bool,
    current: &ItemBuilder,
    current_field: &mut Option<&'static str>,
    listings: &mut Vec<RawListing>,
) {
    let tag = std::str::from_utf8(name).unwrap_or("");
    if tag == "item" && *in_item {
        *in_item = false;
        if let Some(listing) = build_listing(current) {
            listings.push(listing);
        } else {
            warn!(
                link = %current.link,
                title = %current.title,
                "indeed_rss: skipping malformed item",
            );
        }
    } else if matches!(tag, "title" | "link" | "description" | "guid" | "pubDate") {
        *current_field = None;
    }
}

/// `quick-xml` 0.38 surfaces XML entity refs (`&amp;`, `&lt;`, numeric
/// `&#65;` / `&#x41;`) as a separate event rather than splicing them
/// into the surrounding `Event::Text`. Without this handler, every
/// `&`-bearing title or company name (e.g. "Procter & Gamble") would
/// silently lose the entity. Resolve and append into the active field.
fn handle_entity(bytes: &[u8], field: &'static str, current: &mut ItemBuilder) {
    let Ok(name) = std::str::from_utf8(bytes) else {
        warn!(field = %field, "indeed_rss: non-utf8 entity reference; skipping");
        return;
    };
    if let Some(resolved) = resolve_entity_reference(name) {
        push_field(current, field, &resolved);
    } else {
        warn!(field = %field, entity = %name, "indeed_rss: unknown entity reference; skipping");
    }
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

/// Resolve a single XML entity reference (the bytes between `&` and `;`).
///
/// Handles the five XML predefined entities (`amp`, `lt`, `gt`, `apos`,
/// `quot`) plus numeric character references (`#65` decimal,
/// `#x41`/`#X41` hex). Unknown named entities return `None` so the caller
/// can log and drop them rather than embedding a garbled placeholder.
fn resolve_entity_reference(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix('#') {
        let (radix, digits) = if let Some(hex) = rest.strip_prefix(['x', 'X']) {
            (16, hex)
        } else {
            (10, rest)
        };
        let code = u32::from_str_radix(digits, radix).ok()?;
        let ch = char::from_u32(code)?;
        let mut s = String::with_capacity(ch.len_utf8());
        s.push(ch);
        return Some(s);
    }
    resolve_predefined_entity(name).map(str::to_owned)
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
///   * `"Senior AI/ML - Robotics - Acme Corp - Remote"` (role with hyphens)
///   * `"Senior Engineer"` (rare; no separator)
///
/// Indeed's convention is consistent: the **last** segment is the
/// location (when present) and the **second-to-last** is the company.
/// Anything before that is the role title. This holds even when the
/// role name contains additional `" - "` separators (e.g. job tracks).
/// We deliberately rejoin the role-segments so a role like "Senior
/// AI/ML - Robotics" survives intact rather than being shoved into
/// `company` and clobbering the real employer.
fn split_title(raw: &str) -> (String, String, Option<String>) {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split(" - ").collect();
    match parts.as_slice() {
        [] => (String::new(), String::new(), None),
        [title] => ((*title).trim().to_owned(), String::new(), None),
        [title, company] => (
            (*title).trim().to_owned(),
            (*company).trim().to_owned(),
            None,
        ),
        // 3+ segments: last is location, second-to-last is company,
        // everything before is the role (rejoined with " - ").
        all => {
            let n = all.len();
            let location = all[n - 1].trim().to_owned();
            let company = all[n - 2].trim().to_owned();
            let title = all[..n - 2].join(" - ").trim().to_owned();
            let location = if location.is_empty() {
                None
            } else {
                Some(location)
            };
            (title, company, location)
        }
    }
}

/// SHA256 over `(SOURCE_NAME, url.trim())` truncated to 16 hex chars.
/// Used when the feed item omits `<guid>`.
///
/// The URL is trimmed inside this helper so callers can pass the raw
/// `<link>` text without worrying about leading/trailing whitespace; the
/// listing's `url` field is also stored trimmed (see `build_listing`),
/// so trimming here keeps the dedupe key aligned with the URL the rest
/// of the pipeline sees. Without the trim, an `<link>  https://...  </link>`
/// item would produce a different `external_id` from the same item with
/// no whitespace, breaking dedupe across runs.
fn derive_external_id(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_NAME.as_bytes());
    hasher.update(b"\0");
    hasher.update(url.trim().as_bytes());
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
    fn split_title_keeps_hyphenated_role_intact() {
        // Real-world Indeed shape: role with internal " - " (e.g. job
        // track or seniority). The role must not be clobbered into
        // `company`; the company is always the second-to-last segment.
        let (title, company, loc) =
            split_title("Senior AI/ML - Robotics - Acme Corp - San Francisco, CA");
        assert_eq!(title, "Senior AI/ML - Robotics");
        assert_eq!(company, "Acme Corp");
        assert_eq!(loc.as_deref(), Some("San Francisco, CA"));
    }

    #[test]
    fn split_title_three_parts_treats_last_as_location() {
        let (title, company, loc) = split_title("Senior Engineer - Acme Corp - Remote");
        assert_eq!(title, "Senior Engineer");
        assert_eq!(company, "Acme Corp");
        assert_eq!(loc.as_deref(), Some("Remote"));
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

    #[test]
    fn parse_feed_resolves_xml_entities_in_title_and_description() {
        // Real Indeed feeds embed `&amp;` for any company with a `&` in
        // its name. `quick-xml` 0.38 emits these as a separate
        // `Event::GeneralRef`, not inline in `Event::Text`. Without the
        // GeneralRef arm in `parse_feed`, "Procter & Gamble" would
        // collapse into "ProcterGamble".
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<item>
  <title>Engineer - Procter &amp; Gamble - Remote</title>
  <link>https://www.indeed.com/viewjob?jk=pg</link>
  <description>1 &lt; 2 and we use &quot;Rust&quot; &#65;-grade</description>
  <guid>pg-1</guid>
</item>
</channel></rss>"#;
        let listings = parse_feed(xml);
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].company, "Procter & Gamble");
        assert_eq!(listings[0].title, "Engineer");
        assert_eq!(listings[0].location.as_deref(), Some("Remote"));
        // `<` `>` `"` plus a numeric character reference all survive.
        assert!(
            listings[0]
                .description
                .contains(r#"1 < 2 and we use "Rust" A-grade"#),
            "unexpected description: {}",
            listings[0].description,
        );
    }

    #[test]
    fn parse_feed_drops_unknown_named_entity_without_aborting_item() {
        // An unknown entity reference should be skipped (with a
        // `warn!` log) rather than crash or produce a `&entityName;`
        // literal in the output.
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<item>
  <title>Eng &nbsp; Acme</title>
  <link>https://www.indeed.com/viewjob?jk=z</link>
  <description>ok</description>
  <guid>z</guid>
</item>
</channel></rss>"#;
        let listings = parse_feed(xml);
        assert_eq!(listings.len(), 1);
        // Whatever the heuristic does with the resulting title, it must
        // not contain a literal `&nbsp;` or `&` followed by a name.
        assert!(
            !listings[0].title.contains("&nbsp;") && !listings[0].title.contains('&'),
            "unknown entity should be dropped, got: {}",
            listings[0].title,
        );
    }

    #[test]
    fn resolve_entity_reference_handles_named_and_numeric() {
        assert_eq!(resolve_entity_reference("amp").as_deref(), Some("&"));
        assert_eq!(resolve_entity_reference("lt").as_deref(), Some("<"));
        assert_eq!(resolve_entity_reference("apos").as_deref(), Some("'"));
        assert_eq!(resolve_entity_reference("#65").as_deref(), Some("A"));
        assert_eq!(resolve_entity_reference("#x41").as_deref(), Some("A"));
        assert_eq!(resolve_entity_reference("#X41").as_deref(), Some("A"));
        assert!(resolve_entity_reference("nope").is_none());
        assert!(resolve_entity_reference("#xZZZ").is_none());
        assert!(resolve_entity_reference("#999999999").is_none());
    }

    #[tokio::test]
    async fn discover_clamps_out_of_range_fromage() {
        // `fromage = 9999` must be clamped to 30 before hitting the
        // upstream — and the request must still succeed. We assert via
        // wiremock's `query_param` matcher that the clamped value is
        // what was sent.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rss"))
            .and(query_param("fromage", "30"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_XML))
            .mount(&server)
            .await;

        let cfg = IndeedRssSourceConfig {
            enabled: true,
            keywords: "x".into(),
            location: None,
            fromage: Some(9999),
            rate_per_minute: 0,
        };
        let src = IndeedRssSource::new(cfg).with_base_url(server.uri());
        src.discover().await.expect("clamped fromage must succeed");
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

    #[test]
    fn rate_limiter_is_shared_across_constructions() {
        // Regression: `careerai-pipeline::build_sources` reconstructs
        // `IndeedRssSource` on every cron tick. Without the interner the
        // rate limiter would be rebuilt with a full bucket each tick and
        // `rate_per_minute` would be unenforced in daemon mode. Two calls
        // with the same source name must return the same limiter `Arc`
        // (pointer equality).
        //
        // The interner is a process-wide `OnceLock`, shared with the
        // production `SOURCE_NAME` key and any other test in this module
        // that constructs an `IndeedRssSource`. Use a test-unique name so
        // we control the cache state and can assert a non-None limiter
        // without depending on test execution order.
        let a = intern_rate_limiter("indeed_rss-intern-test", 30);
        let b = intern_rate_limiter("indeed_rss-intern-test", 30);
        let arc_a = a.as_ref().expect("limiter set for non-zero rate");
        let arc_b = b.as_ref().expect("limiter set for non-zero rate");
        assert!(
            Arc::ptr_eq(arc_a, arc_b),
            "rate_limiter Arc must be shared across IndeedRssSource constructions",
        );
    }

    #[test]
    fn derive_external_id_ignores_link_whitespace() {
        // Regression: `<link>` text with leading/trailing whitespace must
        // hash to the same external_id as the trimmed form, otherwise the
        // listing's `url` (stored trimmed) and `external_id` would
        // diverge across runs and dedupe would break.
        let trimmed = derive_external_id("https://www.indeed.com/viewjob?jk=abc123");
        let padded = derive_external_id("  https://www.indeed.com/viewjob?jk=abc123  \n");
        assert_eq!(
            trimmed, padded,
            "derive_external_id must trim before hashing",
        );
    }
}
