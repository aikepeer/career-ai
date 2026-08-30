//! Clap struct definitions + tracing init extracted from `main.rs`.
//! Kept separate so `main.rs` (the entry point + dispatcher) stays
//! under the project's 300-LOC cap.

use clap::{Parser, Subcommand};

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
    /// Pull new listings from configured sources. Prints the id of each
    /// new listing for use with `careerai tailor <id>`.
    Discover {
        #[arg(long = "source", value_delimiter = ',')]
        sources: Vec<String>,
    },
    /// Run filters + embedding match against shortlisted listings.
    Match {
        #[arg(long)]
        tune: bool,
        /// Re-score already-shortlisted listings and demote any that fall
        /// below the current threshold back to filtered_out. Useful after
        /// raising score_threshold in local.yaml.
        #[arg(long = "rematch-shortlisted")]
        rematch_shortlisted: bool,
    },
    /// Run the full pipeline (match → tailor → render → apply) in one shot.
    Run {
        #[arg(long = "auto-submit")]
        auto_submit: bool,
    },
    /// Tailor the master resume to a shortlisted listing.
    Tailor {
        listing_id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Render a tailored application to DOCX + PDF via pandoc.
    Render {
        application_id: Option<String>,
        #[arg(long)]
        all: bool,
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
    /// Reset a failed application to its pre-submit state and retry apply.
    Retry {
        application_id: String,
    },
    /// Rollback a listing/application state (e.g. rendered -> tailored, tailored -> shortlisted).
    Rollback {
        id: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long = "from")]
        from_state: Option<String>,
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
    /// List submitted applications that have gone quiet (no response
    /// for N days) — follow-up candidates.
    Followups {
        /// Minimum quiet days before an application is listed.
        #[arg(long, default_value_t = 10)]
        days: u32,
    },
    /// Verify shortlisted listings are still open on their source board
    /// before spending LLM tokens tailoring them.
    Liveness {
        /// Restrict the check to one source (e.g. `greenhouse`).
        #[arg(long)]
        source: Option<String>,
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
    /// Benchmark a company's salary against your own `salary_data.json`.
    Salary {
        /// Company name to look up (fuzzy-matched).
        company: Option<String>,
        /// Narrow the lookup to a city.
        #[arg(long)]
        city: Option<String>,
        /// Emit raw JSON instead of the table.
        #[arg(long)]
        json: bool,
        /// List every company in the data file.
        #[arg(long = "list-all")]
        list_all: bool,
        /// Validate the data file's shape and report duplicates.
        #[arg(long)]
        validate: bool,
        /// Compare your profile's compensation target against the market
        /// index for the matched company.
        #[arg(long)]
        gap: bool,
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
    use std::fs::OpenOptions;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{fmt, EnvFilter, Layer};

    let filter = match log_flag {
        Some(level) => EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info")),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };

    // Logs live under the resolved career-ai root, never the invocation
    // CWD — `careerai` can be run from anywhere and runit services do not
    // share the developer's working directory.
    let log_dir = careerai_core::paths::resolve_root_env()
        .join("data")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file_path = log_dir.join("careerai.log");

    // The log may capture listing text and LLM output; keep it owner-only
    // like the credentials file (mode 0600 from creation, no umask window).
    let file_layer = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&log_file_path)
        }
        #[cfg(not(unix))]
        {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path)
        }
    }
    .ok()
    .map(|file| {
        fmt::layer()
            .with_writer(file)
            .with_ansi(false)
            .with_target(true)
            .with_filter(filter.clone())
    });

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();
}
