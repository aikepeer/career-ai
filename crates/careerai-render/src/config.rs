//! Typed slice of `CoreConfig` for this crate.
//!
//! `RenderConfig` lives on `careerai_core::config::CoreConfig::render`
//! (additive field, `#[serde(default)]`). It is re-exported here so the
//! render crate can be used without reaching across crates for the type.

pub use careerai_core::config::RenderConfig;
