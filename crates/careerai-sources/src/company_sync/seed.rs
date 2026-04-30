//! Seed data: `SeedEntry`, `AtsVendor`, `EMBEDDED_SEED_YAML`, and
//! `load_embedded_seed`. Pure data — no async, no HTTP, no scoring.

use serde::{Deserialize, Serialize};

/// Embedded seed list, parsed from
/// `crates/careerai-sources/src/templates/seed_companies.yaml` at
/// compile time.
pub const EMBEDDED_SEED_YAML: &str = include_str!("../templates/seed_companies.yaml");

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("seed parse: {0}")]
    SeedParse(#[from] serde_yaml::Error),
}

/// One ATS vendor. Stable on the wire — used as a YAML tag in the seed
/// file, so `kebab-case` matches the existing config style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AtsVendor {
    Greenhouse,
    Lever,
    Ashby,
}

impl AtsVendor {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
        }
    }
}

/// One row in `seed_companies.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedEntry {
    pub slug: String,
    pub ats: AtsVendor,
    /// Optional. Currently informational; the probe scores against
    /// every domain regardless. Future use: skip probes for domains
    /// we know in advance won't match.
    #[serde(default)]
    pub domain_hint: Vec<String>,
}

/// Parse the embedded `seed_companies.yaml`. Convenience wrapper so
/// callers don't have to repeat the `serde_yaml::from_str` call.
pub fn load_embedded_seed() -> Result<Vec<SeedEntry>, SyncError> {
    let entries: Vec<SeedEntry> = serde_yaml::from_str(EMBEDDED_SEED_YAML)?;
    Ok(entries)
}
