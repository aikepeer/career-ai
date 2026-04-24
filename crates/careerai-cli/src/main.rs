//! `careerai` CLI entry point.
//!
//! Subcommands dispatch into the workspace crates. `init`, `profile import`,
//! `profile show`, `profile validate`, and `--help` are wired end-to-end at
//! M1; the rest are stubs until M2+.

mod pipeline;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use careerai_core::config::CoreConfig;

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
        /// Overwrite an existing `profile/profile.yaml`.
        #[arg(long)]
        force: bool,
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

    let cwd = std::env::current_dir()?;
    match cli.command {
        Command::Init { force } => {
            careerai_core::init::scaffold(&cwd, force)?;
        }
        Command::Profile { command } => run_profile(command)?,
        Command::Discover { sources } => run_discover(&cwd, &sources).await?,
        Command::Match { tune } => run_match(&cwd, tune).await?,
        Command::Shortlist { command } => match command {
            ShortlistCommand::Show { limit } => run_shortlist_show(&cwd, limit).await?,
        },
        Command::Tailor { .. }
        | Command::Render { .. }
        | Command::Apply { .. }
        | Command::Daemon
        | Command::Inspect { .. }
        | Command::Applied => {
            anyhow::bail!("subcommand not implemented yet (tracked in plan milestones M3+)");
        }
    }
    Ok(())
}

fn load_cfg(cwd: &Path) -> Result<CoreConfig> {
    CoreConfig::load(cwd).context("load config")
}

async fn run_discover(cwd: &Path, source_filter: &[String]) -> Result<()> {
    let cfg = load_cfg(cwd)?;
    let report = pipeline::discover_all(cwd, &cfg, source_filter).await?;
    println!(
        "discover: fetched {}, new {}, duplicates {}, errors {}",
        report.fetched, report.new_rows, report.duplicates, report.errors,
    );
    Ok(())
}

async fn run_match(cwd: &Path, tune: bool) -> Result<()> {
    let cfg = load_cfg(cwd)?;
    let report = pipeline::match_all(cwd, &cfg, tune).await?;
    if tune {
        println!(
            "match --tune: {} listings after filters (filtered_out: {})",
            report.histogram.iter().map(|(_, c)| c).sum::<usize>(),
            report.filtered_out,
        );
        println!("score distribution:");
        for (lower, count) in report.histogram {
            let bar = "#".repeat(count.min(60));
            println!("  [{:.1}-{:.1}) {:>4} {bar}", lower, lower + 0.1, count);
        }
        println!(
            "threshold in config: {:.2} — use match (without --tune) to apply",
            cfg.matching.score_threshold,
        );
    } else {
        println!(
            "match: filtered_out {}, shortlisted {}, below-threshold {}",
            report.filtered_out, report.shortlisted, report.also_filtered,
        );
    }
    Ok(())
}

async fn run_shortlist_show(cwd: &Path, limit: u32) -> Result<()> {
    let rows = pipeline::shortlist_show(cwd, i64::from(limit)).await?;
    if rows.is_empty() {
        println!("(no shortlisted listings — run `careerai discover` then `careerai match`)");
        return Ok(());
    }
    for (i, l) in rows.iter().enumerate() {
        let score = l
            .score
            .map_or_else(|| "—".to_string(), |s| format!("{s:.3}"));
        println!(
            "{:>2}. [{score}] {} @ {} ({})\n    {}",
            i + 1,
            l.title,
            l.company,
            l.source,
            l.url,
        );
    }
    Ok(())
}

/// Default location for the canonical profile file: `./profile/profile.yaml`.
fn profile_yaml_path() -> Result<PathBuf> {
    Ok(std::env::current_dir()?
        .join("profile")
        .join("profile.yaml"))
}

fn run_profile(command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::Import { paths, force } => profile_import(&paths, force),
        ProfileCommand::Show => profile_show(),
        ProfileCommand::Validate => profile_validate(),
    }
}

fn profile_import(paths: &[PathBuf], force: bool) -> Result<()> {
    if paths.is_empty() {
        anyhow::bail!("profile import: at least one source file is required");
    }
    let out = profile_yaml_path()?;
    if out.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite",
            out.display(),
        );
    }
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let profile = careerai_profile::import_paths(&refs).context("parsing profile sources")?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let yaml = profile.to_yaml().context("serialize profile")?;
    std::fs::write(&out, &yaml).with_context(|| format!("write {}", out.display()))?;
    tracing::info!(out = %out.display(), bytes = yaml.len(), "profile imported");
    println!("wrote {}", out.display());
    Ok(())
}

fn profile_show() -> Result<()> {
    let out = profile_yaml_path()?;
    let text = std::fs::read_to_string(&out).with_context(|| format!("read {}", out.display()))?;
    println!("{text}");
    Ok(())
}

fn profile_validate() -> Result<()> {
    let out = profile_yaml_path()?;
    let text = std::fs::read_to_string(&out).with_context(|| format!("read {}", out.display()))?;
    let profile = careerai_profile::Profile::from_yaml(&text).context("parse profile yaml")?;
    profile.check().context("profile validation failed")?;
    println!("profile ok: {}", out.display());
    Ok(())
}
