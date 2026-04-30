//! LinkedIn browser-driven discovery source config.

use serde::{Deserialize, Serialize};

use super::default_true;

/// LinkedIn browser-driven discovery source. Spawns a stealth Chromium
/// session via the shared `BrowserSession` infrastructure (M5), reuses
/// the `li_at` cookie from the OS keychain (service `career-ai`, user
/// `linkedin/li_at`), and scrapes job cards off the public search page.
///
/// `Source::name()` returns `"linkedin-browser"` so the scheduler can
/// schedule it independently of the M5 LinkedIn submitter, but each
/// `RawListing` it emits sets `source = "linkedin"` so `submit_application`
/// routes it to `LinkedinSubmitter` without a special case.
///
/// Defaults to `enabled: false` because LinkedIn's User Agreement
/// section 8.2 forbids automated access. The user opts in once they've
/// accepted that risk — same posture as the M5 submitter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedinBrowserSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Search keywords. Joined with spaces and URL-encoded into the
    /// `keywords=` query parameter on `/jobs/search/`.
    #[serde(default)]
    pub keywords: String,
    /// LinkedIn location string. `"Worldwide"` is the documented value
    /// for the worldwide-remote bucket; country names also work.
    #[serde(default = "default_linkedin_location")]
    pub location: String,
    /// Refinement filters surfaced as URL query parameters.
    #[serde(default)]
    pub filters: LinkedinBrowserFilters,
    /// Hard cap on pagination. Each page is ~25 cards; default 3 pages
    /// = ~75 listings per tick. Values above
    /// `careerai_sources::linkedin_browser::MAX_PAGES_CEILING` are
    /// clamped at adapter construction with a warn log — LinkedIn
    /// CAPTCHA-walls high page counts and a triggered challenge
    /// invalidates the shared `li_at` cookie used by the M5 submitter.
    #[serde(default = "default_linkedin_max_pages")]
    pub max_pages: u32,
    /// Calls per minute soft cap on the page-load rate. `0` disables.
    /// Independent of `rates.linkedin.*` (which sizes the write-side
    /// submit cap). Default 2/minute is conservative — search-result
    /// pages are noticeably less aggressive than the JD-view rate
    /// LinkedIn enforces on logged-in scraping.
    #[serde(default = "default_linkedin_rate_per_minute")]
    pub rate_per_minute: u32,
    /// Run Chromium headlessly. Default `true`. Operators debugging a
    /// selector drift can flip to `false` to watch the scraper drive.
    #[serde(default = "default_true")]
    pub headless: bool,
    /// Per-CDP-request timeout in seconds for navigations and waits.
    /// Default 30s.
    #[serde(default = "default_linkedin_action_timeout")]
    pub action_timeout_seconds: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkedinBrowserFilters {
    /// Filter to remote-only postings (`f_WT=2`). Default `false`.
    #[serde(default)]
    pub remote: bool,
    /// Posted-within window in days. Mapped to `f_TPR=r<seconds>`.
    /// Default `None` (no filter).
    #[serde(default)]
    pub posted_within_days: Option<u32>,
    /// Experience level filters (`f_E=`). Accepts the LinkedIn
    /// taxonomy strings: `internship`, `entry`, `associate`, `mid`,
    /// `senior`, `director`, `executive`. Unknown values are ignored
    /// at adapter construction with a warn log.
    #[serde(default)]
    pub experience_level: Vec<String>,
}

impl Default for LinkedinBrowserSourceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keywords: String::new(),
            location: default_linkedin_location(),
            filters: LinkedinBrowserFilters::default(),
            max_pages: default_linkedin_max_pages(),
            rate_per_minute: default_linkedin_rate_per_minute(),
            headless: true,
            action_timeout_seconds: default_linkedin_action_timeout(),
        }
    }
}

fn default_linkedin_location() -> String {
    "Worldwide".to_string()
}

fn default_linkedin_max_pages() -> u32 {
    3
}

fn default_linkedin_rate_per_minute() -> u32 {
    2
}

fn default_linkedin_action_timeout() -> u64 {
    30
}
