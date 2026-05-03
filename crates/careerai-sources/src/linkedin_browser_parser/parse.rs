use scraper::{ElementRef, Html, Selector};
use tracing::warn;

use crate::base::RawListing;
use crate::linkedin_browser_selectors as sel;
use crate::util::html_to_text;

/// `RawListing.source` value emitted by this parser.
pub const LISTING_SOURCE: &str = "linkedin";

/// Public LinkedIn jobs search base URL.
pub const SEARCH_BASE: &str = "https://www.linkedin.com/jobs/search/";

/// Whitelist for `f_E=` experience-level filter values.
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
