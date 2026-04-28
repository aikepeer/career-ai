//! Pure HTML → `RawListing` parser for LinkedIn job-search pages.
//!
//! Feature-flag-FREE so the offline regression tests + this module's
//! own unit tests run on the default `cargo test -p careerai-sources`
//! build, without pulling in chromiumoxide. The runtime browser path
//! in `linkedin_browser.rs` (gated on `feature = "browser"`) feeds
//! the CDP-rendered HTML into this same parser, so a selector drift
//! caught by the offline test reflects the live behavior.

use scraper::{ElementRef, Html, Selector};
use tracing::warn;

use crate::base::RawListing;
use crate::linkedin_browser_selectors as sel;
use crate::util::html_to_text;
use careerai_core::config::{LinkedinBrowserFilters, LinkedinBrowserSourceConfig};

/// `RawListing.source` value emitted by this parser. Set to
/// `"linkedin"` (NOT `"linkedin-browser"`) so the M5 submitter routes
/// downstream applications to `LinkedinSubmitter` without a
/// special-case in `submit_application`.
pub const LISTING_SOURCE: &str = "linkedin";

/// Public LinkedIn jobs search base URL.
pub const SEARCH_BASE: &str = "https://www.linkedin.com/jobs/search/";

/// Whitelist for `f_E=` experience-level filter values. Anything
/// outside this set is dropped at adapter construction with a warn
/// log so a config typo never silently scrapes the wrong bucket.
pub const KNOWN_EXPERIENCE_LEVELS: &[(&str, &str)] = &[
    ("internship", "1"),
    ("entry", "2"),
    ("associate", "3"),
    ("mid", "4"),
    ("senior", "5"),
    ("director", "6"),
    ("executive", "7"),
];

/// Parse a captured LinkedIn search-results page into `RawListing`s.
///
/// Pure: no I/O, no browser. Malformed cards (missing title, missing
/// href) are skipped. Cards with a title but no company / no snippet
/// emit a listing with empty strings rather than panicking — downstream
/// consumers (filter, embed) tolerate empty fields.
#[must_use]
pub fn parse_search_html(html: &str) -> Vec<RawListing> {
    let doc = Html::parse_document(html);
    let card_sel = match Selector::parse(sel::JOB_CARD) {
        Ok(s) => s,
        Err(e) => {
            warn!(target: "linkedin-browser", error = %e, "JOB_CARD selector parse failed");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for card in doc.select(&card_sel) {
        if let Some(listing) = parse_one_card(&card) {
            out.push(listing);
        }
    }
    out
}

fn parse_one_card(card: &ElementRef<'_>) -> Option<RawListing> {
    let title_anchor = first_match(card, sel::CARD_TITLE_ANCHOR)?;
    let title = clean_text(&title_anchor.text().collect::<String>());
    let href = title_anchor.value().attr("href").unwrap_or("").to_string();
    if title.is_empty() || href.is_empty() {
        return None;
    }
    let url = absolutize_url(&href);
    let external_id = extract_listing_id(&url, card)?;

    let company = first_match(card, sel::CARD_COMPANY)
        .map(|el| clean_text(&el.text().collect::<String>()))
        .unwrap_or_default();
    let location = first_match(card, sel::CARD_LOCATION)
        .map(|el| clean_text(&el.text().collect::<String>()))
        .filter(|s| !s.is_empty());
    let posted_time = first_match(card, sel::CARD_POSTED_TIME)
        .map(|el| clean_text(&el.text().collect::<String>()))
        .filter(|s| !s.is_empty());
    let snippet = first_match(card, sel::CARD_SNIPPET)
        .map(|el| el.inner_html())
        .map(|h| html_to_text(&h))
        .unwrap_or_default();

    let description = match posted_time.as_deref() {
        Some(when) if !snippet.is_empty() => format!("[posted {when}] {snippet}"),
        Some(when) => format!("[posted {when}]"),
        None => snippet,
    };

    let raw_json = serde_json::to_string(&serde_json::json!({
        "title": title,
        "company": company,
        "location": location,
        "url": url,
        "posted": posted_time,
    }))
    .ok();

    Some(RawListing {
        source: LISTING_SOURCE.to_string(),
        external_id,
        title,
        company,
        location,
        url,
        description,
        raw_json,
    })
}

/// Extract a stable LinkedIn listing id from the apply URL or card
/// data attribute. Tries:
///   1. `data-entity-urn` attribute (`urn:li:jobPosting:1234567`).
///   2. Inner element with `data-job-id`.
///   3. Path segment `/jobs/view/<id>` of the URL.
fn extract_listing_id(url: &str, card: &ElementRef<'_>) -> Option<String> {
    if let Some(urn) = card.value().attr("data-entity-urn") {
        if let Some(id) = urn.rsplit(':').next() {
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    if let Ok(inner_sel) = Selector::parse("[data-job-id]") {
        if let Some(el) = card.select(&inner_sel).next() {
            if let Some(id) = el.value().attr("data-job-id") {
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    if let Some(after) = url.split("/jobs/view/").nth(1) {
        let id: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

fn absolutize_url(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if let Some(rest) = href.strip_prefix('/') {
        format!("https://www.linkedin.com/{rest}")
    } else {
        format!("https://www.linkedin.com/{href}")
    }
}

fn first_match<'a>(parent: &ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    let s = Selector::parse(css).ok()?;
    parent.select(&s).next()
}

fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a `linkedin.com/jobs/search/?...` URL for one page.
///
/// LinkedIn paginates via the `start=` query param (one card-batch
/// per 25 increments). Page 0 omits `start`; page N sets `start = N*25`.
#[must_use]
pub fn build_search_url(cfg: &LinkedinBrowserSourceConfig, page_idx: u32) -> String {
    let mut q: Vec<(String, String)> = Vec::new();
    if !cfg.keywords.trim().is_empty() {
        q.push(("keywords".to_string(), cfg.keywords.clone()));
    }
    if !cfg.location.trim().is_empty() {
        q.push(("location".to_string(), cfg.location.clone()));
    }
    apply_filter_params(&cfg.filters, &mut q);
    if page_idx > 0 {
        q.push(("start".to_string(), (page_idx * 25).to_string()));
    }
    if q.is_empty() {
        return SEARCH_BASE.to_string();
    }
    let qs = q
        .iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{SEARCH_BASE}?{qs}")
}

fn apply_filter_params(filters: &LinkedinBrowserFilters, out: &mut Vec<(String, String)>) {
    if filters.remote {
        // LinkedIn's "remote" workplace-type code is `2`.
        out.push(("f_WT".to_string(), "2".to_string()));
    }
    if let Some(days) = filters.posted_within_days {
        let seconds = u64::from(days) * 86_400;
        out.push(("f_TPR".to_string(), format!("r{seconds}")));
    }
    if !filters.experience_level.is_empty() {
        let codes: Vec<&str> = filters
            .experience_level
            .iter()
            .filter_map(|lvl| {
                let lvl_lc = lvl.to_ascii_lowercase();
                KNOWN_EXPERIENCE_LEVELS
                    .iter()
                    .find(|(name, _)| *name == lvl_lc)
                    .map(|(_, code)| *code)
            })
            .collect();
        if !codes.is_empty() {
            out.push(("f_E".to_string(), codes.join(",")));
        }
    }
}

/// Minimal application/x-www-form-urlencoded encoder. Only space and
/// the URL-reserved chars need escaping for LinkedIn's search params.
pub fn urlencode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let safe = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if safe {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            // write! into a String never errors; ignore the Result.
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encodes_spaces_as_plus() {
        assert_eq!(urlencode("AI engineer"), "AI+engineer");
    }

    #[test]
    fn url_encodes_special_chars() {
        assert_eq!(urlencode("c++"), "c%2B%2B");
    }

    #[test]
    fn build_url_page_zero_omits_start() {
        let cfg = LinkedinBrowserSourceConfig {
            keywords: "AI engineer".to_string(),
            location: "Worldwide".to_string(),
            ..LinkedinBrowserSourceConfig::default()
        };
        let url = build_search_url(&cfg, 0);
        assert!(url.contains("keywords=AI+engineer"), "got: {url}");
        assert!(url.contains("location=Worldwide"), "got: {url}");
        assert!(
            !url.contains("start="),
            "page 0 should omit start, got: {url}"
        );
    }

    #[test]
    fn build_url_pagination_steps_by_25() {
        let cfg = LinkedinBrowserSourceConfig::default();
        assert!(build_search_url(&cfg, 1).contains("start=25"));
        assert!(build_search_url(&cfg, 2).contains("start=50"));
        assert!(build_search_url(&cfg, 3).contains("start=75"));
    }

    #[test]
    fn build_url_remote_filter_emits_f_wt_2() {
        let cfg = LinkedinBrowserSourceConfig {
            filters: LinkedinBrowserFilters {
                remote: true,
                ..LinkedinBrowserFilters::default()
            },
            ..LinkedinBrowserSourceConfig::default()
        };
        assert!(build_search_url(&cfg, 0).contains("f_WT=2"));
    }

    #[test]
    fn build_url_posted_within_days_emits_f_tpr_seconds() {
        let cfg = LinkedinBrowserSourceConfig {
            filters: LinkedinBrowserFilters {
                posted_within_days: Some(7),
                ..LinkedinBrowserFilters::default()
            },
            ..LinkedinBrowserSourceConfig::default()
        };
        // 7 * 86400 = 604800
        assert!(build_search_url(&cfg, 0).contains("f_TPR=r604800"));
    }

    #[test]
    fn build_url_experience_levels_map_to_codes() {
        let cfg = LinkedinBrowserSourceConfig {
            filters: LinkedinBrowserFilters {
                experience_level: vec!["mid".to_string(), "senior".to_string()],
                ..LinkedinBrowserFilters::default()
            },
            ..LinkedinBrowserSourceConfig::default()
        };
        let url = build_search_url(&cfg, 0);
        // mid=4, senior=5
        assert!(url.contains("f_E="), "got: {url}");
        assert!(url.contains('4') && url.contains('5'), "got: {url}");
    }

    #[test]
    fn parse_empty_html_returns_empty() {
        let listings = parse_search_html("<html><body></body></html>");
        assert!(listings.is_empty());
    }

    #[test]
    fn extract_id_from_urn() {
        let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item" data-entity-urn="urn:li:jobPosting:9876543"><a class="base-card__full-link" href="/jobs/view/9876543/">Title</a></li></ul>"#;
        let listings = parse_search_html(html);
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "9876543");
    }

    #[test]
    fn extract_id_from_url_when_urn_missing() {
        let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item"><a class="base-card__full-link" href="/jobs/view/1234567/?refId=abc">Job Title</a></li></ul>"#;
        let listings = parse_search_html(html);
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].external_id, "1234567");
        assert_eq!(listings[0].source, "linkedin");
    }

    #[test]
    fn malformed_card_with_no_title_skipped_not_panicked() {
        let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item" data-entity-urn="urn:li:jobPosting:1"></li></ul>"#;
        let listings = parse_search_html(html);
        assert!(listings.is_empty());
    }

    #[test]
    fn missing_company_yields_empty_string_not_panic() {
        let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item" data-entity-urn="urn:li:jobPosting:55"><a class="base-card__full-link" href="/jobs/view/55/">A Role</a></li></ul>"#;
        let listings = parse_search_html(html);
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].title, "A Role");
        assert_eq!(listings[0].company, "");
    }
}
