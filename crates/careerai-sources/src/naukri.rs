//! Naukri.com job board source.
//!
//! Endpoint: `GET {base}/jobapi/v3/search?keywords=<kw>&location=<loc>&...`
//!
//! This is Naukri's undocumented internal SPA endpoint. Headers
//! `AppId: 109` / `SystemId: 109` are required — Naukri's public web client
//! sends them on every request and the API rejects callers that don't.
//! They are **not** secrets; both values are visible in the browser's
//! network panel on any naukri.com page load.
//!
//! Off by default in `SourcesConfig::NaukriSourceConfig::default`. User
//! opts in by flipping `sources.naukri.enabled = true` in
//! `config/local.yaml`.
//!
//! ## Module layout
//!
//! * [`source`] — [`NaukriSource`] driver: HTTP client, discovery, URL
//!   absolutization.
//! * [`types`] — response types ([`Payload`], [`JobDetail`],
//!   [`Placeholder`]), constants, and the string-or-number deserializer.

mod source;
#[cfg(test)]
mod tests;
mod types;

pub use source::NaukriSource;
