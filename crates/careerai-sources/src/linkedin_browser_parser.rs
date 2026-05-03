//! Pure HTML → `RawListing` parser for LinkedIn job-search pages.
//!
//! Feature-flag-FREE so offline regression tests + unit tests run on
//! the default `cargo test -p careerai-sources` build, without pulling
//! in chromiumoxide.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod parse;
#[cfg(test)]
mod tests;
mod url;

pub use parse::{parse_search_html, KNOWN_EXPERIENCE_LEVELS, LISTING_SOURCE, SEARCH_BASE};
pub use url::{build_search_url, urlencode};
