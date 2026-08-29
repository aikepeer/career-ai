//! LinkedIn job-search DOM selectors as `const &str`.
//!
//! Kept feature-flag-FREE (no `cfg(feature = "browser")`) so the
//! offline `scraper`-based regression test in
//! `tests/linkedin_browser_fixture_it.rs` can import the SAME
//! selectors the runtime path uses, even when the workspace is built
//! without `--features browser` (CI default). When LinkedIn renames a
//! class, the fixture test breaks BEFORE the live scraper does — same
//! posture as `linkedin_selectors.rs` in `careerai-submit`.
//!
//! ## What to monitor for drift
//!
//! Any of these selectors going stale shows up as "0 listings" in the
//! cron log. The fixture test's `expected_listings_parsed_from_fixture`
//! catches it pre-merge.
//!
//! Update path:
//!   1. Capture a fresh `linkedin.com/jobs/search/...` page (logged
//!      in, sanitize PII), drop the HTML in
//!      `tests/fixtures/linkedin_search_p1.html`.
//!   2. Update the constants below to match the new class names.
//!   3. Run `cargo test -p careerai-sources linkedin_browser` and
//!      iterate.

/// Search-results outer list. Used as the "page is loaded" signal —
/// the adapter waits until at least one card is rendered under this
/// container before scraping.
pub const RESULTS_LIST: &str = "ul.jobs-search__results-list, ul.scaffold-layout__list-container";

/// One card per job. The adapter iterates against this selector and
/// pulls the per-card sub-fields with the selectors below.
///
/// Only `<li>` variants are matched (NOT `div.base-card` on its own)
/// because logged-in LinkedIn pages render an outer `<li>` with an
/// inner `<div class="base-card">`. Matching both would yield the
/// same listing twice, collapsing to one row via the
/// `(source, external_id)` upsert downstream but inflating the
/// per-tick "cards scraped" telemetry by 2×.
pub const JOB_CARD: &str =
    "li:has(div.base-card), li.jobs-search-results__list-item, li.scaffold-layout__list-item, li.base-card, div.job-search-card";

/// Listing title anchor. The `href` is the apply URL; the text is the
/// job title.
pub const CARD_TITLE_ANCHOR: &str =
    "a.base-card__full-link, a.job-card-list__title, h3 a, a.job-card-container__link";

/// Company name. LinkedIn flips between `<h4>` and `<span>` based on
/// the result-card variant.
pub const CARD_COMPANY: &str =
    "h4.base-search-card__subtitle, span.job-card-container__primary-description, a.hidden-nested-link";

/// Posting location.
pub const CARD_LOCATION: &str =
    "span.job-search-card__location, span.job-card-container__metadata-item, .base-search-card__metadata span";

/// Posted-time label. Plain-text only; no parsing happens here.
pub const CARD_POSTED_TIME: &str =
    "time.job-search-card__listdate, time.job-search-card__listdate--new, time";

/// Snippet text — usually 1–2 lines describing the role. Often
/// missing on logged-out search; we tolerate that.
pub const CARD_SNIPPET: &str =
    "p.job-search-card__snippet, div.base-search-card__snippet, .base-search-card__metadata + p";
