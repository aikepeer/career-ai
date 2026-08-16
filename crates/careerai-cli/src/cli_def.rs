//! Clap struct definitions + tracing init extracted from `main.rs`.
//! Kept separate so `main.rs` (the entry point + dispatcher) stays
//! under the project's 300-LOC cap.

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::commands::{
    ConfigSubcommand, CookiesCommand, LlmCommand, McpCommand, NotifyCommand, ProfileCommand,
    ServiceCommand, ShortlistCommand, SourcesCommand, StatusCommand,
};

#[derive(Debug, Parser)]
#[command(
    name = "careerai",
    version,
    about = "Automated job discovery, resume tailoring, and auto-apply",
    long_about = None,
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Set log verbosity (overrides RUST_LOG).
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log: Option<String>,

    /// Override the LLM backend selection (`auto`, `claude-cli`, `api`, `agy`,
    /// `codex`, `pi`, `goose`, `grok`, `aider`, `copilot`, `llama-cpp`, or
    /// custom binary path like `~/.local/bin/agy`).
    #[arg(
        long = "llm-backend",
        global = true,
        value_name = "auto|claude-cli|api|agy|codex|pi|goose|grok|aider|copilot|<path>"
    )]
    pub llm_backend: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate or update configuration file `config/local.yaml`.
    Config {
        #[command(subcommand)]
        command: ConfigSubcommand,
    },
    /// Scaffold `config/`, `profile/`, and `.env` in the current directory.
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Pull new listings from configured sources.
    Discover {
        #[arg(long = "source", value_delimiter = ',')]
        sources: Vec<String>,
    },
    /// Run filters + embedding match against shortlisted listings.
    Match {
        #[arg(long)]
        tune: bool,
    },
    /// Tailor the master resume to a shortlisted listing.
    Tailor {
        listing_id: String,
    },
    /// Render a tailored application to DOCX + PDF via pandoc.
    Render {
        application_id: String,
    },
    /// Submit prepared applications. Defaults to dry-run.
    Apply {
        application_id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long = "auto-submit")]
        auto_submit: bool,
        #[arg(long)]
        source: Option<String>,
    },
    /// Start the long-running scheduler daemon.
    Daemon,
    /// Inspect an application's row, state history, and artifacts.
    Inspect {
        application_id: String,
    },
    /// Profile ingestion + validation subcommands.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Show shortlisted listings.
    Shortlist {
        #[command(subcommand)]
        command: ShortlistCommand,
    },
    /// Show applications submitted so far.
    Applied {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    Review,
    Cookies {
        #[command(subcommand)]
        command: CookiesCommand,
    },
    /// Print a daily summary of pipeline activity.
    Digest {
        #[arg(long, default_value = "24h")]
        since: String,
    },
    /// Tools for MCP-server discovery sources.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Inspect or probe the LLM backend.
    Llm {
        #[command(subcommand)]
        command: LlmCommand,
    },
    /// Notification pipeline tools.
    Notify {
        #[command(subcommand)]
        command: NotifyCommand,
    },
    /// Manage discovery sources.
    Sources {
        #[command(subcommand)]
        command: SourcesCommand,
    },
    /// Show the pipeline dashboard.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
    /// Manage the systemd user service.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

pub(crate) fn init_tracing(log_flag: Option<&str>) {
    let filter = match log_flag {
        Some(level) => EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info")),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
