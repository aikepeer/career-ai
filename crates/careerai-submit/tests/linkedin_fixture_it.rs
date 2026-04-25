//! Selector regression test for the LinkedIn Easy Apply CTA.
//!
//! No browser, no chromiumoxide. We parse a captured LinkedIn
//! job-view HTML fixture with `scraper` and assert that exactly one
//! element matches the same CSS selector `linkedin.rs::click_easy_apply`
//! uses. When LinkedIn renames the button class, this test breaks
//! BEFORE the live browser flow does — operators get a clear signal
//! to update the selector + fixture in one commit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_submit::linkedin_selectors::EASY_APPLY_SELECTOR;
use scraper::{Html, Selector};

const LINKEDIN_FIXTURE_HTML: &str = r#"
<!doctype html>
<html><body>
  <header><h1>Senior ML Engineer</h1></header>
  <div class="jobs-apply">
    <button class="jobs-apply-button artdeco-button artdeco-button--3"
            aria-label="Easy Apply to Senior ML Engineer at Acme">
      Easy Apply
    </button>
  </div>
</body></html>
"#;

const LINKEDIN_NO_EASY_APPLY_FIXTURE: &str = r#"
<!doctype html>
<html><body>
  <header><h1>Senior ML Engineer</h1></header>
  <div class="jobs-apply">
    <a class="jobs-apply-button artdeco-button"
       href="https://acme.example.com/apply"
       aria-label="Apply on company site">
      Apply on company site
    </a>
  </div>
</body></html>
"#;

#[test]
fn easy_apply_selector_matches_button_only() {
    let doc = Html::parse_document(LINKEDIN_FIXTURE_HTML);
    let sel = Selector::parse(EASY_APPLY_SELECTOR).unwrap();
    let matches: Vec<_> = doc.select(&sel).collect();
    assert_eq!(matches.len(), 1, "expected exactly one Easy Apply button");
    let el = &matches[0];
    assert_eq!(el.value().name(), "button");
    let aria = el.value().attr("aria-label").unwrap_or_default();
    assert!(
        aria.contains("Easy Apply"),
        "aria-label should mention Easy Apply, got: {aria:?}"
    );
}

#[test]
fn selector_does_not_match_external_apply_link() {
    // The fallback flow ('Apply on company site') uses an <a>, not a
    // <button>. Our selector is `button.jobs-apply-button` so the
    // <a> must NOT match — otherwise the live path would click an
    // off-site link instead of opening Easy Apply.
    let doc = Html::parse_document(LINKEDIN_NO_EASY_APPLY_FIXTURE);
    let sel = Selector::parse(EASY_APPLY_SELECTOR).unwrap();
    let matches: Vec<_> = doc.select(&sel).collect();
    assert!(
        matches.is_empty(),
        "selector must not match <a> tag for off-site apply"
    );
}
