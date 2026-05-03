#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use careerai_core::config::IndeedRssSourceConfig;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::parser::{derive_external_id, parse_feed, resolve_entity_reference, split_title};
use super::source::{intern_rate_limiter, IndeedRssSource};
use crate::base::{Source, SourceError};

const SAMPLE_XML: &str = include_str!("../../tests/fixtures/indeed_rss_sample.xml");

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
