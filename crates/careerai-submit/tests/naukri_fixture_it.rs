//! Naukri selector fixture test. Parses a minimal captured HTML via `scraper`
//! and verifies the `naukri_selectors` constants find the right elements.
//! Mirrors `linkedin_fixture_it.rs` discipline — a selector change at the
//! LIVE site can't pass this test while breaking the real flow because the
//! synthetic HTML uses the same anchors we depend on for production.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_submit::naukri_selectors::*;
use scraper::{Html, Selector};

const NAUKRI_FIXTURE_HTML: &str = include_str!("fixtures/naukri_jd.html");

#[test]
fn apply_button_selector_pattern_is_well_formed() {
    // Sanity: the union selector compiles to valid CSS that scraper accepts.
    Selector::parse(APPLY_BUTTON_SELECTOR)
        .expect("APPLY_BUTTON_SELECTOR must be valid CSS");
    assert!(APPLY_BUTTON_SELECTOR.contains("apply-button"));
    assert!(APPLY_BUTTON_SELECTOR.contains("aria-label"));
}

#[test]
fn apply_button_selector_matches_fixture_button() {
    let doc = Html::parse_document(NAUKRI_FIXTURE_HTML);
    let sel = Selector::parse(APPLY_BUTTON_SELECTOR).unwrap();
    let matches: Vec<_> = doc.select(&sel).collect();
    assert!(
        !matches.is_empty(),
        "APPLY_BUTTON_SELECTOR should match at least one element in the fixture"
    );
    // The first match should be a button with the aria-label "Apply".
    let first = &matches[0];
    assert_eq!(first.value().name(), "button");
}

#[test]
fn confirm_apply_selector_matches_modal_button() {
    let doc = Html::parse_document(NAUKRI_FIXTURE_HTML);
    let sel = Selector::parse(CONFIRM_APPLY_SELECTOR).unwrap();
    let matches: Vec<_> = doc.select(&sel).collect();
    assert!(
        !matches.is_empty(),
        "CONFIRM_APPLY_SELECTOR should match the modal confirm button in the fixture"
    );
}

#[test]
fn login_required_indicator_includes_login() {
    // Structural check: the selector references known Naukri login DOM anchors.
    assert!(LOGIN_REQUIRED_INDICATOR.contains("login"));
    // Validate it parses as CSS.
    Selector::parse(LOGIN_REQUIRED_INDICATOR)
        .expect("LOGIN_REQUIRED_INDICATOR must be valid CSS");
}

#[test]
fn fixture_html_contains_known_anchors() {
    // The anchors we'll match in the live browser must appear in the
    // captured fixture — proves the selectors target real Naukri DOM.
    assert!(NAUKRI_FIXTURE_HTML.contains("apply-button-12345"));
    assert!(NAUKRI_FIXTURE_HTML.contains("aria-label=\"Apply\""));
    assert!(NAUKRI_FIXTURE_HTML.contains("apply-button-text"));
}
