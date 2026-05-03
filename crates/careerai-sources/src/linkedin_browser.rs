//! Native LinkedIn browser-driven discovery adapter.
//!
//! Drives a stealth Chromium session against the public
//! `linkedin.com/jobs/search/` page, paginates over results, and
//! emits one `RawListing` per job card.
//!
//! Feature-gated on `browser`. The default build never pulls in
//! chromiumoxide or careerai-submit.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

#![cfg(feature = "browser")]

mod scrape;
mod source;
#[cfg(test)]
mod tests;

pub use source::{LinkedinBrowserSource, MAX_PAGES_CEILING};
