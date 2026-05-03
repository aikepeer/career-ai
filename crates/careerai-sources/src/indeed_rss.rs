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

mod parser;
mod source;
#[cfg(test)]
mod tests;

pub use source::IndeedRssSource;
