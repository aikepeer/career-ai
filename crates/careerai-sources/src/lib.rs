//! Job-board discovery adapters.
//!
//! One implementation per board behind the [`base::Source`] trait. New
//! boards plug in by implementing `Source`; callers never branch on the
//! concrete type.

pub mod ashby;
pub mod base;
pub mod company_sync;
pub mod greenhouse;
pub mod indeed_rss;
pub mod lever;
#[cfg(feature = "browser")]
pub mod linkedin_browser;
pub mod linkedin_browser_parser;
pub mod linkedin_browser_selectors;
pub mod mcp_jobs;
pub mod naukri;
pub mod remoteok;
pub mod remotive;
pub mod util;
pub mod web_agent;

pub use web_agent::{DiscoveredPortal, WebSearchDiscoveryAgent};

pub use ashby::AshbySource;
pub use base::{RawListing, Source, SourceError};
pub use company_sync::{
    load_embedded_seed, sync as sync_companies, AtsVendor, CompanyHit, SeedEntry, SyncError,
    SyncReport,
};
pub use greenhouse::GreenhouseSource;
pub use indeed_rss::IndeedRssSource;
pub use lever::LeverSource;
#[cfg(feature = "browser")]
pub use linkedin_browser::LinkedinBrowserSource;
pub use mcp_jobs::{probe as probe_mcp_source, McpJobsSource, ProbeReport};
pub use naukri::NaukriSource;
pub use remoteok::RemoteOkSource;
pub use remotive::RemotiveSource;
