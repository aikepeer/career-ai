//! `careerai` CLI entry point.
//!
//! Subcommands dispatch into the workspace crates. Most are stubs at M0;
//! only `init` and `--help` are wired end-to-end.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "careerai",
    version,
    about = "Automated job discovery, resume tailoring, and auto-apply",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Set log verbosity (overrides RUST_LOG).
    #[arg(long, global = true, value_name = "LEVEL")]
    log: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scaffold `config/`, `profile/`, and `.env` in the current directory.
    Init {
        /// Overwrite existing files.
        #[arg(long)]
        force: bool,
    },
    /// Pull new listings from configured sources.
    Discover {
        /// Restrict to one or more sources (repeatable).
        #[arg(long = "source")]
        sources: Vec<String>,
    },
    /// Run filters + embedding match against shortlisted listings.
    Match {
        /// Print score distribution instead of persisting matches.
        #[arg(long)]
        tune: bool,
    },
    /// Tailor the master resume to a shortlisted listing.
    Tailor {
        /// Listing ID (UUID) to tailor for.
        listing_id: String,
    },
    /// Render a tailored application to DOCX + PDF via pandoc.
    Render {
        /// Application ID (UUID) to render.
        application_id: String,
    },
    /// Submit prepared applications. Defaults to dry-run.
    Apply {
        /// Apply to every shortlisted + tailored application.
        #[arg(long)]
        all: bool,
        /// Actually submit instead of dry-run.
        #[arg(long = "auto-submit")]
        auto_submit: bool,
        /// Single application ID to apply for.
        application_id: Option<String>,
    },
    /// Start the long-running scheduler daemon.
    Daemon,
    /// Inspect a listing, application, or the last pipeline run.
    Inspect {
        /// Show the most recent run summary.
        #[arg(long = "last-run")]
        last_run: bool,
        /// Show details for an application by ID.
        application_id: Option<String>,
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
    Applied,
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    /// Import resume + LinkedIn export into `profile/profile.yaml`.
    Import {
        /// One or more source files (PDF, DOCX, or LinkedIn ZIP).
        paths: Vec<std::path::PathBuf>,
    },
    /// Print the parsed profile.
    Show,
    /// Validate `profile/profile.yaml` against the schema.
    Validate,
}

#[derive(Debug, Subcommand)]
enum ShortlistCommand {
    /// List shortlisted listings.
    Show {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

fn init_tracing(log_flag: Option<&str>) {
    let filter = match log_flag {
        Some(level) => EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info")),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log.as_deref());

    match cli.command {
        Command::Init { force } => careerai_core::init::scaffold(&std::env::current_dir()?, force)?,
        Command::Discover { .. }
        | Command::Match { .. }
        | Command::Tailor { .. }
        | Command::Render { .. }
        | Command::Apply { .. }
        | Command::Daemon
        | Command::Inspect { .. }
        | Command::Profile { .. }
        | Command::Shortlist { .. }
        | Command::Applied => {
            anyhow::bail!("subcommand not implemented yet (tracked in plan milestones M1+)");
        }
    }
    Ok(())
}
