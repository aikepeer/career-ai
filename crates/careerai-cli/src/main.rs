//! `careerai` CLI entry point.
//!
//! Subcommands dispatch into the workspace crates. `init`, `profile import`,
//! `profile show`, `profile validate`, and `--help` are wired end-to-end at
//! M1; the rest are stubs until M2+.

use careerai_pipeline as pipeline;

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
        /// Single application ID to apply for. If omitted, `--all` is required.
        application_id: Option<String>,
        /// Apply to every application in state `rendered` or `prepared`.
        #[arg(long)]
        all: bool,
        /// Force live submission (default is dry-run regardless of config).
        #[arg(long = "auto-submit")]
        auto_submit: bool,
        /// Restrict `--all` to a single source (greenhouse, lever, ashby, ...).
        #[arg(long)]
        source: Option<String>,
    },
    /// Start the long-running scheduler daemon.
    Daemon,
    /// Inspect an application's row, state history, and artifacts.
    Inspect {
        /// Application ID (UUID) to inspect.
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
        /// Filter by the listing's source.
        #[arg(long)]
        source: Option<String>,
        /// Maximum rows to return (newest first).
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
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
        Command::Tailor { listing_id } => {
            let cfg = load_cfg(&cwd)?;
            match pipeline::tailor_one(&cwd, &cfg, &listing_id).await {
                Ok(outcome) => {
                    println!(
                        "tailored: application_id={} ({} @ {})",
                        outcome.application_id, outcome.listing_title, outcome.company,
                    );
                    println!(
                        "run `careerai render {}` to emit DOCX/PDF artifacts",
                        outcome.application_id,
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "tailor failed");
                    std::process::exit(map_tailor_error_to_exit_code(&e));
                }
            }
        }
        Command::Render { application_id } => {
            let cfg = load_cfg(&cwd)?;
            match pipeline::render_one(&cwd, &cfg, &application_id).await {
                Ok(outcome) => {
                    println!("rendered: application_id={}", outcome.application_id);
                    for (path, size) in &outcome.bytes {
                        println!("  {} ({} bytes)", path.display(), size);
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "render failed");
                    std::process::exit(map_render_error_to_exit_code(&e));
                }
            }
        }
        Command::Apply {
            application_id,
            all,
            auto_submit,
            source,
        } => {
            let cfg = load_cfg(&cwd)?;
            run_apply(
                &cwd,
                &cfg,
                application_id,
                all,
                auto_submit,
                source.as_deref(),
            )
            .await?;
        }
        Command::Applied { source, limit } => {
            run_applied(&cwd, source.as_deref(), limit).await?;
        }
        Command::Inspect { application_id } => {
            run_inspect(&cwd, &application_id).await?;
        }
        Command::Daemon => {
            let cfg = load_cfg(&cwd)?;
            let sched = careerai_scheduler::Scheduler::from_config(&cwd, &cfg)
                .await
                .context("init scheduler")?;
            sched
                .run_until_shutdown()
                .await
                .context("scheduler shutdown")?;
        }
    }
    Ok(())
}

/// Dispatch for `careerai apply`. Errors short-circuit the process with a
/// typed exit code; success paths print one line per application.
async fn run_apply(
    cwd: &Path,
    cfg: &CoreConfig,
    application_id: Option<String>,
    all: bool,
    auto_submit: bool,
    source_filter: Option<&str>,
) -> Result<()> {
    let auto_submit_override = if auto_submit { Some(true) } else { Some(false) };

    match (application_id, all) {
        (None, false) => {
            anyhow::bail!("specify --all or an application id");
        }
        (Some(_), true) => {
            anyhow::bail!("pass either --all or an application id, not both");
        }
        (Some(id), false) => match pipeline::apply_one(cwd, cfg, &id, auto_submit_override).await {
            Ok(outcome) => {
                print_apply_line(&outcome, auto_submit);
            }
            Err(e) => {
                tracing::error!(error = %e, "apply failed");
                std::process::exit(map_apply_error_to_exit_code(&e));
            }
        },
        (None, true) => {
            match pipeline::apply_all(cwd, cfg, source_filter, auto_submit_override).await {
                Ok(outcomes) => {
                    if outcomes.is_empty() {
                        println!(
                            "(no rendered/prepared applications — run `careerai render` first)"
                        );
                        return Ok(());
                    }
                    for outcome in &outcomes {
                        print_apply_line(outcome, auto_submit);
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "apply --all failed");
                    std::process::exit(map_apply_error_to_exit_code(&e));
                }
            }
        }
    }
    Ok(())
}

fn print_apply_line(outcome: &pipeline::AppliedOutcome, auto_submit: bool) {
    let tag = if auto_submit {
        "[live]   "
    } else {
        "[dry-run]"
    };
    match &outcome.outcome {
        careerai_submit::SubmitOutcome::DryRun { .. } => {
            println!(
                "{tag} application={} source={} -> DryRun (would_submit logged)",
                outcome.application_id, outcome.source,
            );
        }
        careerai_submit::SubmitOutcome::Submitted { remote_id } => {
            println!(
                "[live]    application={} source={} -> Submitted (remote={})",
                outcome.application_id, outcome.source, remote_id,
            );
        }
        careerai_submit::SubmitOutcome::Skipped { reason } => {
            println!(
                "[skip]    application={} source={} -> Skipped ({})",
                outcome.application_id, outcome.source, reason,
            );
        }
    }
}

async fn run_applied(cwd: &Path, source_filter: Option<&str>, limit: i64) -> Result<()> {
    let rows = pipeline::applied_show(cwd, source_filter, limit).await?;
    if rows.is_empty() {
        println!("(no submitted applications yet)");
        return Ok(());
    }
    // Columnar header.
    println!(
        "{:<38}  {:<12}  {:<10}  UPDATED_AT",
        "APPLICATION_ID", "SOURCE", "STATE"
    );
    // We need the listing's source per row; fetch once per row.
    let pool = pipeline::open_pool(cwd).await?;
    for app in rows {
        let source = careerai_db::queries::find_by_id(&pool, &app.listing_id)
            .await
            .map_or_else(|_| "?".to_owned(), |l| l.source);
        println!(
            "{:<38}  {:<12}  {:<10}  {}",
            app.id,
            source,
            app.state,
            app.updated_at.format("%Y-%m-%dT%H:%M:%SZ"),
        );
    }
    Ok(())
}

async fn run_inspect(cwd: &Path, application_id: &str) -> Result<()> {
    match pipeline::inspect_show(cwd, application_id).await {
        Ok(report) => {
            println!("Application  : {}", report.application.id);
            println!("State        : {}", report.application.state);
            println!(
                "Listing      : {} @ {} (source={})",
                report.listing_title, report.listing_company, report.listing_source,
            );
            println!("Profile hash : {}", report.application.profile_hash);
            println!("Prompt ver   : {}", report.application.prompt_version);
            println!("LLM model    : {}", report.application.llm_model);
            println!("Events:");
            for e in &report.events {
                let ts = e.created_at.format("%Y-%m-%dT%H:%M:%SZ");
                match &e.note {
                    Some(note) => println!("  {ts} {} ({note})", e.to_state),
                    None => println!("  {ts} {}", e.to_state),
                }
            }
            println!("Artifacts:");
            if report.artifacts.is_empty() {
                println!("  (none)");
            } else {
                for a in &report.artifacts {
                    println!("  {:<14} {} ({} bytes)", a.kind, a.path, a.bytes);
                }
            }
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, "inspect failed");
            std::process::exit(map_apply_error_to_exit_code(&e));
        }
    }
}

/// Map an apply/inspect-path error to a stable exit code.
///
/// - 2: application / listing not found
/// - 3: application in wrong state (not 'rendered'/'prepared')
/// - 5: source disabled (per-source config gate)
/// - 6: ATS upstream HTTP failure
/// - 7: unknown source (no submitter registered)
/// - 1: anything else
fn map_apply_error_to_exit_code(err: &anyhow::Error) -> i32 {
    // Walk the cause chain and match on typed errors only. The
    // earlier stringly-typed fallback (`msg.starts_with("application
    // not found:")`) was fragile: any `.context("…")` wrapping
    // prepended text and silently broke the match. The typed downcast
    // path covers every real path because `apply_one` / `inspect_show`
    // wrap `careerai_db::DbError::NotFound` directly, and
    // `submit_application` returns `careerai_submit::SubmitError`.
    for cause in err.chain() {
        if let Some(se) = cause.downcast_ref::<careerai_submit::SubmitError>() {
            return match se {
                careerai_submit::SubmitError::BadState { .. } => 3,
                careerai_submit::SubmitError::SourceDisabled(_) => 5,
                careerai_submit::SubmitError::UnknownSource(_) => 7,
                careerai_submit::SubmitError::Http(_)
                | careerai_submit::SubmitError::HttpStatus { .. } => 6,
                careerai_submit::SubmitError::Db(careerai_db::DbError::NotFound(_)) => 2,
                _ => 1,
            };
        }
        if let Some(careerai_db::DbError::NotFound(_)) =
            cause.downcast_ref::<careerai_db::DbError>()
        {
            return 2;
        }
    }
    1
}

/// Map a tailor-path error into a stable process exit code.
///
/// - 2: listing / application not found
/// - 3: listing in unexpected state
/// - 4: constrained-diff validator rejected the LLM output
/// - 5: LLM provider/upstream failure
/// - 1: anything else
fn map_tailor_error_to_exit_code(err: &anyhow::Error) -> i32 {
    // Walk the chain looking for typed causes.
    for cause in err.chain() {
        if let Some(te) = cause.downcast_ref::<careerai_tailor::TailorError>() {
            return match te {
                careerai_tailor::TailorError::InventedContent { .. }
                | careerai_tailor::TailorError::Schema(_)
                | careerai_tailor::TailorError::BadPath(_)
                | careerai_tailor::TailorError::CoverLetterTooLong { .. } => 4,
                careerai_tailor::TailorError::Llm(_) => 5,
                _ => 1,
            };
        }
        if cause.downcast_ref::<careerai_llm::LlmError>().is_some() {
            return 5;
        }
    }

    // String-level fallbacks for the `anyhow::bail!` paths that never carry a
    // typed cause — keep these in sync with the messages in `pipeline.rs`.
    let msg = err.to_string();
    if msg.starts_with("listing not found:") || msg.starts_with("application not found:") {
        return 2;
    }
    if msg.contains("expected 'shortlisted'") {
        return 3;
    }
    1
}

/// Map a render-path error into a stable process exit code.
///
/// - 2: application / payload / listing not found
/// - 3: application in unexpected state
/// - 6: pandoc missing on PATH
/// - 1: anything else
fn map_render_error_to_exit_code(err: &anyhow::Error) -> i32 {
    for cause in err.chain() {
        if let Some(re) = cause.downcast_ref::<careerai_render::RenderError>() {
            if matches!(re, careerai_render::RenderError::PandocMissing) {
                return 6;
            }
        }
    }

    let msg = err.to_string();
    if msg.starts_with("application not found:")
        || msg.starts_with("application payload not found:")
        || msg.starts_with("listing not found:")
    {
        return 2;
    }
    if msg.contains("expected 'tailored'") {
        return 3;
    }
    1
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
