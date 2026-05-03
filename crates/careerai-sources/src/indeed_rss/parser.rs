//! RSS 2.0 XML parser. Walks `quick-xml` events, resolves entities,
//! builds [`RawListing`]s from `<item>` entries.

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::base::RawListing;
use crate::util::html_to_text;

use super::source::SOURCE_NAME;

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
pub(crate) fn parse_feed(xml: &str) -> Vec<RawListing> {
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
pub(crate) fn resolve_entity_reference(name: &str) -> Option<String> {
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
pub(crate) fn split_title(raw: &str) -> (String, String, Option<String>) {
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
pub(crate) fn derive_external_id(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_NAME.as_bytes());
    hasher.update(b"\0");
    hasher.update(url.trim().as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}
