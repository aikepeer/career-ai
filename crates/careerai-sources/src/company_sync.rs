//! Auto-discover ATS companies whose currently-open jobs match the
//! user's `domains:` keywords.
//!
//! `careerai sources sync` runs this module against a seed list of
//! known-public Greenhouse / Lever / Ashby slugs (embedded via
//! `include_str!` from `templates/seed_companies.yaml`), probes each
//! board's public API, scores every JD against every domain in
//! `CoreConfig.domains`, and produces a three-way diff against the
//! currently-configured `sources.<ats>.companies` lists.
//!
//! Public surface:
//! - [`SyncReport`] — the diff (`add` / `keep` / `remove` /
//!   `probe_failures`).
//! - [`CompanyHit`] — one matched company entry.
//! - [`AtsVendor`] — Greenhouse | Lever | Ashby.
//! - [`SeedEntry`] — the on-disk schema of `seed_companies.yaml`.
//! - [`load_embedded_seed`] — convenience parser for the bundled list.
//! - [`sync`] — the entry point.
//!
//! Concurrency: up to [`PROBE_CONCURRENCY`] companies are probed in
//! parallel. Each probe has a [`PROBE_TIMEOUT`] hard ceiling. HTTP
//! errors are soft-failed (logged, recorded in
//! `report.probe_failures`); the sync never aborts on a single
//! upstream blip.

mod partition;
mod probe;
mod seed;
#[cfg(test)]
mod tests;

pub use partition::{sync, sync_with_base_urls, SyncReport};
pub use probe::{BaseUrls, CompanyHit, PROBE_CONCURRENCY, PROBE_TIMEOUT};
pub use seed::{load_embedded_seed, AtsVendor, SeedEntry, SyncError, EMBEDDED_SEED_YAML};
