//! Integration test for the native LinkedIn browser-discovery
//! adapter's parser path.
//!
//! This test runs without `--features browser`. It feeds a captured
//! (synthetic) LinkedIn search-results HTML page through the SAME
//! `parse_search_html` function the runtime browser path consumes,
//! exercising the public selectors the live scraper uses
//! (`linkedin_browser_selectors`). Per CLAUDE.md's testing-layers
//! rule, this is the "Browser → Golden / snapshot" layer simplified
//! to skip the chromiumoxide spawn — the parser is pure, so a
//! browser launch would only test chromiumoxide itself.
//!
//! When LinkedIn drifts, this test breaks BEFORE the live cron path
//! does. Update both the fixture and the selectors in lockstep.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_sources::linkedin_browser_parser::parse_search_html;

const FIXTURE_HTML: &str = include_str!("fixtures/linkedin_search_p1.html");

#[test]
fn parses_at_least_five_listings_from_fixture() {
    let listings = parse_search_html(FIXTURE_HTML);
    assert!(
        listings.len() >= 5,
        "expected at least 5 valid listings parsed from fixture, got {}: {:#?}",
        listings.len(),
        listings.iter().map(|l| &l.external_id).collect::<Vec<_>>(),
    );
}

#[test]
fn every_listing_emits_listing_source_linkedin() {
    let listings = parse_search_html(FIXTURE_HTML);
    for l in &listings {
        // The Source::name() is "linkedin-browser" but listings
        // stored in DB MUST set source = "linkedin" so the M5
        // submitter routes them.
        assert_eq!(
            l.source, "linkedin",
            "expected source=linkedin (so submitter routes), got {:?} on listing id={}",
            l.source, l.external_id
        );
    }
}

#[test]
fn every_listing_has_required_fields() {
    let listings = parse_search_html(FIXTURE_HTML);
    for l in &listings {
        assert!(
            !l.external_id.is_empty(),
            "external_id empty on listing: {l:?}"
        );
        assert!(
            !l.title.is_empty(),
            "title empty on listing id={}",
            l.external_id
        );
        assert!(
            l.url.starts_with("https://www.linkedin.com/jobs/view/"),
            "url should be absolute LinkedIn job-view URL, got {:?} on id={}",
            l.url,
            l.external_id
        );
    }
}

#[test]
fn external_ids_are_unique() {
    let listings = parse_search_html(FIXTURE_HTML);
    let mut ids: Vec<&str> = listings.iter().map(|l| l.external_id.as_str()).collect();
    ids.sort_unstable();
    let total = ids.len();
    ids.dedup();
    assert_eq!(
        ids.len(),
        total,
        "duplicate external_ids would collapse via (source, external_id) upsert"
    );
}

#[test]
fn known_titles_present_in_fixture() {
    let listings = parse_search_html(FIXTURE_HTML);
    let titles: Vec<&str> = listings.iter().map(|l| l.title.as_str()).collect();
    assert!(
        titles.contains(&"Senior Machine Learning Engineer"),
        "expected ML role from fixture, got: {titles:?}"
    );
    assert!(
        titles.contains(&"Robotics Software Engineer (Perception)"),
        "expected robotics role from fixture, got: {titles:?}"
    );
}

#[test]
fn missing_company_yields_empty_string_not_skipped() {
    // Card 5 (id=1000000005) has a title + URL but no <h4> subtitle.
    // The parser must still emit it (with company = "") rather than
    // skip — operator value is preserved; downstream filters tolerate
    // empty company.
    let listings = parse_search_html(FIXTURE_HTML);
    let card5 = listings
        .iter()
        .find(|l| l.external_id == "1000000005")
        .expect("listing with id=1000000005 should be present despite missing company");
    assert_eq!(card5.company, "", "company should be empty string");
    assert_eq!(card5.title, "AI Engineer (Stealth)");
}

#[test]
fn malformed_card_with_no_anchor_is_skipped() {
    // Card 6 (id=1000000006) has urn but no anchor / title — must
    // not panic, must not emit.
    let listings = parse_search_html(FIXTURE_HTML);
    assert!(
        listings.iter().all(|l| l.external_id != "1000000006"),
        "card with no anchor must be skipped; got: {:?}",
        listings.iter().map(|l| &l.external_id).collect::<Vec<_>>()
    );
}

#[test]
fn falls_back_to_url_id_when_urn_missing() {
    // Card 7 (id=1000000007) has no data-entity-urn but a
    // /jobs/view/1000000007/ URL — extract_listing_id should fall
    // through to the URL-path branch.
    let listings = parse_search_html(FIXTURE_HTML);
    let card7 = listings
        .iter()
        .find(|l| l.external_id == "1000000007")
        .expect("listing with URL-derived id=1000000007 should be present");
    assert_eq!(card7.title, "ML Platform Engineer");
    assert_eq!(card7.company, "Lattice Compute");
}

#[test]
fn description_includes_posted_time_marker() {
    let listings = parse_search_html(FIXTURE_HTML);
    let card1 = listings
        .iter()
        .find(|l| l.external_id == "1000000001")
        .expect("ML role card present");
    assert!(
        card1.description.contains("[posted "),
        "expected [posted ...] marker in description, got: {:?}",
        card1.description
    );
}

#[test]
fn html_in_snippet_is_stripped() {
    // Card 1 has <strong>Remote-first</strong> in the snippet. The
    // parser routes through util::html_to_text, so the tags must be
    // stripped before storage (downstream embedders + filters expect
    // plain text).
    let listings = parse_search_html(FIXTURE_HTML);
    let card1 = listings
        .iter()
        .find(|l| l.external_id == "1000000001")
        .expect("ML role card present");
    assert!(
        !card1.description.contains("<strong>"),
        "HTML tags should be stripped, got: {:?}",
        card1.description
    );
    assert!(
        card1.description.contains("Remote-first"),
        "tag-stripped text should still contain word, got: {:?}",
        card1.description
    );
}

#[test]
fn empty_html_input_returns_empty_listings() {
    let listings = parse_search_html("");
    assert!(listings.is_empty());
}

#[test]
fn html_with_no_search_list_returns_empty() {
    let listings = parse_search_html("<html><body><p>nothing here</p></body></html>");
    assert!(listings.is_empty());
}
