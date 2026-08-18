//! Subcommand handlers extracted from `main.rs`.
//!
//! Each module owns the dispatch path for one CLI subcommand. The
//! `Command` enum + clap parsing + `main()` dispatch live in
//! `main.rs`; the actual handlers live here so each file stays under
//! the project's 300-LOC cap.
//!
//! Sub-command enums (`ProfileCommand`, `ShortlistCommand`, etc.) are
//! defined here and re-exported so `main.rs` can reference them in the
//! top-level `Command` enum without ballooning its line count.

pub mod apply;
pub mod config_cmd;
pub mod discover;
pub mod inspect;
pub mod llm;
pub mod match_;
pub mod mcp;
pub mod notify;
pub mod profile;
pub mod profile_llm;
pub mod render;
pub mod retry;
pub mod run;
pub mod shortlist;
pub mod tailor;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// Generate exhaustive `config/local.yaml` from candidate profile.
    Generate {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// Import resume + LinkedIn export into `profile/profile.yaml`.
    Import {
        /// One or more source files (PDF, DOCX, or LinkedIn ZIP).
        paths: Vec<std::path::PathBuf>,
        /// Overwrite an existing `profile/profile.yaml`.
        #[arg(long)]
        force: bool,
        /// Route PDF/DOCX text through the LLM extractor instead of the
        /// regex heuristic. Auto-detects when omitted only in builds
        /// with the `live-llm` cargo feature: enabled if an Anthropic
        /// key is reachable (keyring or env), disabled otherwise. In
        /// non-`live-llm` builds the heuristic is always used unless
        /// `--use-llm=true` is passed (which then errors clearly).
        /// Pass `--use-llm=false` to force the heuristic.
        #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
        use_llm: Option<bool>,
    },
    /// Print the parsed profile.
    Show,
    /// Validate `profile/profile.yaml` against the schema.
    Validate,
}

#[derive(Debug, Subcommand)]
pub enum ShortlistCommand {
    /// List shortlisted listings.
    Show {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Debug, Subcommand)]
pub enum StatusCommand {
    /// Start the dashboard HTTP server. Defaults to
    /// 127.0.0.1:8787 with a 60s meta-refresh.
    Serve {
        /// Port to bind. Overrides `dashboard.port` in config.
        #[arg(long)]
        port: Option<u16>,
        /// Bind address. Hidden — defaults to 127.0.0.1. A non-loopback
        /// value is refused unless
        /// `CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK=1` is set, because the
        /// surface has no authentication and mutating endpoints.
        #[arg(long, hide = true)]
        bind: Option<std::net::IpAddr>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Install the systemd user unit file for `careerai daemon`.
    Install {
        /// Overwrite an existing unit file if its content differs.
        #[arg(long)]
        force: bool,
    },
    /// Show systemctl --user status for the careerai service.
    Status,
    /// Disable, stop, and remove the systemd user unit file.
    Uninstall,
}

#[derive(Debug, Subcommand)]
pub enum LlmCommand {
    /// Probe which backend `Auto` resolution would pick on this host
    /// and report a brief health status (binary path/version, ping
    /// latency, API-key source).
    Probe,
}

#[derive(Debug, Subcommand)]
pub enum NotifyCommand {
    /// Fire a synthetic `SourceUnreachable` event through every
    /// configured channel. Lets you verify Slack / Telegram / email /
    /// ntfy are wired correctly without waiting for a real event.
    Test,
}

#[derive(Debug, Subcommand)]
pub enum SourcesCommand {
    /// Probe the seeded ATS list against the configured `domains:`
    /// keywords and either preview or merge the new companies into
    /// `config/local.yaml`. Default is preview.
    Sync {
        /// Write the merged lists into `config/local.yaml`. Without
        /// this flag, the diff is printed and no file is touched.
        #[arg(long)]
        apply: bool,
    },
    /// Discover new job portals and freelance platforms via Web Search Agent.
    DiscoverWeb {
        /// Write discovered job and freelance portals into config/local.yaml
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Probe configured `kind: mcp` sources for reachability. Reports
    /// per-source: tool count, whether a known job-search tool name
    /// is advertised, and any spawn / handshake error. Read-only;
    /// never sends a real query and never writes anything sensitive.
    Probe,
}

#[derive(Debug, Subcommand)]
pub enum CookiesCommand {
    /// Refresh the session cookie for a provider (linkedin or naukri).
    ///
    /// Prompts for the cookie value from your browser DevTools and stores
    /// it in the OS keyring. The daemon picks it up on the next apply tick.
    Refresh {
        /// Provider name: linkedin | naukri
        provider: String,
    },
}
