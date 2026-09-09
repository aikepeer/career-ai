//! Web-feed source configs: Remotive, Naukri, Indeed RSS.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotiveSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub category: Option<String>,
}

impl Default for RemotiveSourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            category: Some("software-dev".to_string()),
        }
    }
}

/// Naukri.com — India's largest job board. Uses the undocumented
/// `jobapi/v3/search` endpoint with `AppId`/`SystemId` headers. Defaults
/// to `enabled: false` because the endpoint is unofficial and Naukri
/// aggressively rate-limits unauthenticated callers; the user opts in
/// via `config/local.yaml` once they've decided to accept the trade-off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NaukriSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

impl Default for NaukriSourceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keywords: vec!["machine learning".into(), "llm".into(), "robotics".into()],
            location: Some("Delhi / NCR".into()),
            max_results: Some(20),
        }
    }
}

/// Indeed RSS feed source. Maps to a single `https://rss.indeed.com/rss`
/// query. Defaults to disabled. The keyword/location strings are sent
/// to Indeed verbatim; Indeed handles its own URL encoding when we hand
/// the values to `reqwest`'s query builder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndeedRssSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Search keywords; passed as the `q` query parameter. Empty string
    /// is allowed (Indeed returns a generic feed in that case).
    #[serde(default)]
    pub keywords: String,
    /// Optional location string passed as the `l` parameter. `None`,
    /// an unset YAML key, or `Some("")` / an empty string omits the
    /// parameter entirely so the feed is not location-restricted.
    #[serde(default)]
    pub location: Option<String>,
    /// Optional `fromage` (recency in days) parameter — Indeed accepts
    /// values like `1`, `3`, `7`, `14`. `None` omits the parameter.
    #[serde(default)]
    pub fromage: Option<u32>,
    /// Soft cap on `discover()` calls per minute. `0` disables the
    /// gate. Mirrors `McpSourceConfig.rate_per_minute`; sized for the
    /// read-side, not write-side traffic that lives in `rates.*`.
    #[serde(default)]
    pub rate_per_minute: u32,
}

/// FreeHire aggregator source (`freehire.me`). Public JSON API, no auth:
/// `GET /api/v1/agent/jobs/search` returns normalized postings from ~50
/// ATS platforms, tech-tuned facets included. Defaults to enabled with
/// the project's niche keywords; `remote_only`/`region`/`jobage` map to
/// upstream facets. The upstream service is best-effort (no SLA), so a
/// freehire outage degrades this one source, never the pipeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FreehireSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Full-text query sent as `q` (title, skill, role). Empty string
    /// performs an unfiltered search.
    #[serde(default)]
    pub keywords: String,
    /// Results per page (upstream API limit).
    #[serde(default = "default_freehire_limit")]
    pub limit: usize,
    /// `remote=remote` facet — only fully-remote postings.
    #[serde(default)]
    pub remote_only: bool,
    /// `region=<codes>` facet, comma-separated (e.g. `eu`, `global`,
    /// `none` for unresolved). Upstream owns the vocabulary.
    #[serde(default)]
    pub region: Option<String>,
    /// `jobage=<days>` — only postings from the last N days.
    #[serde(default)]
    pub jobage: Option<u32>,
}

fn default_freehire_limit() -> usize {
    25
}

/// GitHub org-based job collection. Fetches `jobs.md` / `careers.md` from
/// each configured org's `.github` repo. Optional `token` raises the
/// GitHub API rate limit from 60 to 5,000 req/hour.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GithubJobsSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// GitHub org names to scan for job-posting files.
    #[serde(default)]
    pub orgs: Vec<String>,
    /// Optional GitHub personal access token (read-only `public_repo`
    /// scope is sufficient). Falls back to unauthenticated requests.
    #[serde(default)]
    pub token: Option<String>,
}
