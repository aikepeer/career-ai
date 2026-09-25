//! Discovery-source configuration. The umbrella `SourcesConfig` lives
//! here; per-source bag types live in submodules grouped by transport
//! family (web feeds vs. LinkedIn browser vs. MCP servers).

use serde::{Deserialize, Serialize};

mod linkedin;
mod mcp;
mod web;

pub use linkedin::{LinkedinBrowserFilters, LinkedinBrowserSourceConfig};
pub use mcp::{McpQueryConfig, McpSourceConfig, McpTransportConfig};
pub use web::{
    FreehireSourceConfig, GithubJobsSourceConfig, IndeedRssSourceConfig, NaukriSourceConfig,
    RemotiveSourceConfig,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub locations: Vec<String>,
    #[serde(default)]
    pub greenhouse: CompaniesSource,
    #[serde(default)]
    pub lever: CompaniesSource,
    /// Ashby public posting API (`https://api.ashbyhq.com/posting-api/job-board/<slug>`).
    /// Same shape as Greenhouse / Lever — one entry per company slug.
    /// Discovery adapter ships in `careerai-sources` as `AshbySource`;
    /// the `careerai sources sync` subcommand auto-populates this list
    /// by probing seed slugs against the user's `domains:` keywords.
    #[serde(default)]
    pub ashby: CompaniesSource,
    /// Teamtailor careers boards. One entry per company subdomain — e.g.
    /// `synmatchai` → `https://synmatchai.teamtailor.com/jobs.json`.
    /// Discovery adapter ships in `careerai-sources` as
    /// `TeamtailorSource` (JSON Feed 1.1 + schema.org `_jobposting`).
    #[serde(default)]
    pub teamtailor: CompaniesSource,
    #[serde(default)]
    pub remotive: RemotiveSourceConfig,
    #[serde(default)]
    pub remoteok: ToggleSource,
    /// FreeHire aggregator (`freehire.me`) — public REST API, no auth,
    /// tech-tuned facets. `FreehireSource` adapter in `careerai-sources`.
    #[serde(default)]
    pub freehire: FreehireSourceConfig,
    #[serde(default)]
    pub naukri: NaukriSourceConfig,
    /// Indeed public RSS feed (`https://rss.indeed.com/rss?q=&l=&fromage=`).
    /// Stable, ToS-clean, no auth needed. Single configurable feed: one
    /// keyword query + optional location + recency window. Disabled by
    /// default so a fresh install never makes outbound requests unprompted.
    #[serde(default)]
    pub indeed_rss: IndeedRssSourceConfig,
    /// GitHub org-based job collection. Many companies post openings in a
    /// `jobs.md` / `careers.md` file inside their `.github` repo. This
    /// source fetches those files via the GitHub REST API. One entry per
    /// GitHub org name. Optional `token` for higher rate limits.
    #[serde(default, alias = "github-jobs")]
    pub github_jobs: GithubJobsSourceConfig,
    /// Native LinkedIn browser-driven discovery source. Drives a stealth
    /// Chromium session against `linkedin.com/jobs/search/` using the
    /// same `li_at` cookie + stealth-v2.js infrastructure as the M5
    /// submitter. Defaults to `enabled: false`; user opts in after
    /// acknowledging the LinkedIn ToS §8.2 trade-off.
    #[serde(default, alias = "linkedin-browser")]
    pub linkedin_browser: LinkedinBrowserSourceConfig,
    /// Community / third-party Model-Context-Protocol servers used as
    /// discovery sources. Each entry spawns a stdio MCP server process,
    /// calls `tools/list`, and invokes the first matching job-search
    /// tool. Defaults to empty; users opt in by adding entries to
    /// `config/local.yaml`.
    #[serde(default)]
    pub mcp: Vec<McpSourceConfig>,
    /// Structured priority keyword groups (P1: AI/ML/Embedded/Robotics,
    /// P2: Fullstack/Software, P3: STEM/Telecom). Used by the config
    /// generator and match scorer to weight discovery queries.
    #[serde(default)]
    pub priority_keywords: PriorityKeywords,
    /// Job-type and location filters applied globally across all sources.
    #[serde(default)]
    pub filter: FilterConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompaniesSource {
    #[serde(default)]
    pub companies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToggleSource {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ToggleSource {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub(super) fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PriorityKeywords {
    #[serde(default)]
    pub p1_ai_embedded_robotics: Vec<String>,
    #[serde(default)]
    pub p2_fullstack_software: Vec<String>,
    #[serde(default)]
    pub p3_stem_telecom: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default = "default_remote_pref")]
    pub remote_preference: String,
    #[serde(default = "default_contract_type")]
    pub contract_type: String,
    #[serde(default)]
    pub locations: Vec<String>,
    #[serde(default)]
    pub excluded_companies: Vec<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            remote_preference: default_remote_pref(),
            contract_type: default_contract_type(),
            locations: Vec::new(),
            excluded_companies: Vec::new(),
        }
    }
}

pub(super) fn default_remote_pref() -> String {
    "remote_first".to_string()
}

pub(super) fn default_contract_type() -> String {
    "all".to_string()
}
