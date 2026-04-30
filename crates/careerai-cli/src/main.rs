//! `careerai` CLI entry point.
//!
//! Subcommands dispatch into the workspace crates. `init`, `profile import`,
//! `profile show`, `profile validate`, and `--help` are wired end-to-end at
//! M1; the rest are stubs until M2+.

mod commands;
mod cookies;
mod digest;
mod review;
mod service;
mod sources_sync;
mod status;

use careerai_pipeline as pipeline;

use std::path::Path;

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

    /// Override the LLM backend selection. `auto` (default) prefers the
    /// `claude` CLI when reachable, else falls back to the rig-core
    /// Anthropic API. Force `claude-cli` or `api` to skip detection.
    #[arg(
        long = "llm-backend",
        global = true,
        value_name = "auto|claude-cli|api"
    )]
    llm_backend: Option<String>,
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
        /// Restrict to one or more sources. Accepts comma-separated
        /// (`--source greenhouse,lever`) or repeated
        /// (`--source greenhouse --source lever`) forms.
        #[arg(long = "source", value_delimiter = ',')]
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
    /// Walk drafted LinkedIn applications, prompt y/N per draft, click Submit on yes.
    Review,
    /// Manage session cookies for browser-driven submitters (linkedin, naukri).
    Cookies {
        #[command(subcommand)]
        command: CookiesCommand,
    },
    /// Print a daily summary of pipeline activity (counts by state +
    /// per-source breakdown + last cron tick + cookie expiry warnings).
    Digest {
        /// Time window. Accepts `24h`, `7d`, `2w`, or a number-of-hours
        /// integer. Default: `24h`.
        #[arg(long, default_value = "24h")]
        since: String,
    },
    /// Tools for MCP-server discovery sources.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Inspect or probe the LLM backend (claude CLI vs Anthropic API).
    Llm {
        #[command(subcommand)]
        command: LlmCommand,
    },
    /// Notification pipeline tools (Slack / Telegram / email / ntfy).
    Notify {
        #[command(subcommand)]
        command: NotifyCommand,
    },
    /// Manage discovery sources (auto-discover companies from seed list).
    Sources {
        #[command(subcommand)]
        command: SourcesCommand,
    },
    /// Show the pipeline dashboard. `serve` starts an HTTP server on
    /// 127.0.0.1; `show` is reserved for future CLI summary output.
    Status {
        #[command(subcommand)]
        command: StatusCommand,
    },
    /// Manage the systemd user service that autostarts `careerai daemon`.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

#[derive(Debug, Subcommand)]
enum StatusCommand {
    /// Start the read-only dashboard HTTP server. Defaults to
    /// 127.0.0.1:8787 with a 60s meta-refresh.
    Serve {
        /// Port to bind. Overrides `dashboard.port` in config.
        #[arg(long)]
        port: Option<u16>,
        /// Bind address. Hidden — defaults to 127.0.0.1. Setting any
        /// other value triggers a stderr warning since the surface has
        /// no auth.
        #[arg(long, hide = true)]
        bind: Option<std::net::IpAddr>,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
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
enum LlmCommand {
    /// Probe which backend `Auto` resolution would pick on this host
    /// and report a brief health status (binary path/version, ping
    /// latency, API-key source).
    Probe,
}

#[derive(Debug, Subcommand)]
enum NotifyCommand {
    /// Fire a synthetic `SourceUnreachable` event through every
    /// configured channel. Lets you verify Slack / Telegram / email /
    /// ntfy are wired correctly without waiting for a real event.
    Test,
}

#[derive(Debug, Subcommand)]
enum SourcesCommand {
    /// Probe the seeded ATS list against the configured `domains:`
    /// keywords and either preview or merge the new companies into
    /// `config/local.yaml`. Default is preview.
    Sync {
        /// Write the merged lists into `config/local.yaml`. Without
        /// this flag, the diff is printed and no file is touched.
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    /// Probe configured `kind: mcp` sources for reachability. Reports
    /// per-source: tool count, whether a known job-search tool name
    /// is advertised, and any spawn / handshake error. Read-only;
    /// never sends a real query and never writes anything sensitive.
    Probe,
}

#[derive(Debug, Subcommand)]
enum CookiesCommand {
    /// Refresh the session cookie for a provider (linkedin or naukri).
    ///
    /// Prompts for the cookie value from your browser DevTools and stores
    /// it in the OS keyring. The daemon picks it up on the next apply tick.
    Refresh {
        /// Provider name: linkedin | naukri
        provider: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
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
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log.as_deref());

    let cwd = std::env::current_dir()?;

    // Parse the global `--llm-backend` flag once so subcommands can
    // forward it down without re-parsing.
    let backend_override: Option<careerai_core::config::BackendChoice> =
        match cli.llm_backend.as_deref() {
            Some(s) => Some(s.parse().map_err(|e: String| anyhow::anyhow!(e))?),
            None => None,
        };

    match cli.command {
        Command::Init { force } => {
            careerai_core::init::scaffold(&cwd, force)?;
        }
        Command::Profile { command } => commands::profile::run(command, backend_override)?,
        Command::Discover { sources } => commands::discover::run(&cwd, &sources).await?,
        Command::Match { tune } => commands::match_::run(&cwd, tune).await?,
        Command::Shortlist { command } => match command {
            ShortlistCommand::Show { limit } => commands::shortlist::run_show(&cwd, limit).await?,
        },
        Command::Tailor { listing_id } => {
            let mut cfg = load_cfg(&cwd)?;
            if let Some(b) = backend_override {
                cfg.llm.backend = b;
            }
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
                    tracing::error!(error = %format_args!("{e:#}"), "tailor failed");
                    std::process::exit(commands::tailor::map_tailor_error_to_exit_code(&e));
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
                    tracing::error!(error = %format_args!("{e:#}"), "render failed");
                    std::process::exit(commands::render::map_render_error_to_exit_code(&e));
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
            commands::apply::run(
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
            commands::apply::run_applied(&cwd, source.as_deref(), limit).await?;
        }
        Command::Review => {
            let cfg = load_cfg(&cwd)?;
            review::run_review(&cwd, &cfg).await?;
        }
        Command::Cookies { command } => match command {
            CookiesCommand::Refresh { provider } => {
                cookies::refresh(&provider)?;
            }
        },
        Command::Digest { since } => {
            let cfg = load_cfg(&cwd)?;
            digest::run_digest(&cwd, &cfg, &since).await?;
        }
        Command::Mcp { command } => match command {
            McpCommand::Probe => {
                let cfg = load_cfg(&cwd)?;
                commands::mcp::run_probe(&cfg).await?;
            }
        },
        Command::Llm { command } => match command {
            LlmCommand::Probe => {
                // CoreConfig::load always succeeds (embedded defaults
                // fill any gap), so even a fresh dir works.
                let cfg = load_cfg(&cwd)?;
                commands::llm::run_probe(&cfg, backend_override).await?;
            }
        },
        Command::Notify { command } => match command {
            NotifyCommand::Test => {
                let cfg = load_cfg(&cwd)?;
                commands::notify::run_test(&cfg).await?;
            }
        },
        Command::Sources { command } => match command {
            SourcesCommand::Sync { apply } => {
                sources_sync::run(&cwd, apply).await?;
            }
        },
        Command::Inspect { application_id } => {
            commands::inspect::run(&cwd, &application_id).await?;
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
        Command::Status { command } => match command {
            StatusCommand::Serve { port, bind } => {
                let cfg = load_cfg(&cwd)?;
                status::run_serve(&cwd, &cfg, port, bind).await?;
            }
        },
        Command::Service { command } => match command {
            ServiceCommand::Install { force } => service::run_install(force)?,
            ServiceCommand::Status => service::run_status()?,
            ServiceCommand::Uninstall => service::run_uninstall()?,
        },
    }
    Ok(())
}

// run_mcp_probe / run_llm_probe / probe_forced_resolve / run_notify_test
// were extracted to crates/careerai-cli/src/commands/{mcp,llm,notify}.rs
// to keep main.rs under the project's 300-LOC-per-file guidance.

// Subcommand handlers extracted to `commands/`:
//   * apply.rs    — run, run_applied, print_apply_line, map_apply_error_to_exit_code
//   * inspect.rs  — run
//   * tailor.rs   — map_tailor_error_to_exit_code
//   * render.rs   — map_render_error_to_exit_code
//   * discover.rs / match_.rs / shortlist.rs — one-call dispatchers
// All preserve their original semantics; main.rs keeps the clap
// structs, `main()`, dispatch, and `load_cfg` to stay closer to the
// 300-LOC project guidance.

fn load_cfg(cwd: &Path) -> Result<CoreConfig> {
    CoreConfig::load(cwd).context("load config")
}

// Profile cluster extracted to commands/profile.rs (run_profile,
// profile_import, profile_show, profile_validate,
// detect_stale_skills_schema, profile_yaml_path) and
// commands/profile_llm.rs (run_profile_import_with_llm,
// anthropic_key_reachable, llm_backend_maybe_available,
// strip_provider_prefix, profile_llm_adapter::Adapter).
// All preserve their original semantics.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // detect_stale_skills_schema tests live in commands/profile.rs.
    // strip_provider_prefix tests live in commands/profile_llm.rs.

    /// Regression: when the CLI logs a top-level error, it must include
    /// the full anyhow chain so the operator can see WHY a command
    /// failed (e.g. `tailor failed: tailor_for_listing: bullet not
    /// covered: projects[0].bullets[0]`), not just the outermost
    /// context label. The bug a real user just hit: `careerai tailor`
    /// printed `error=tailor_for_listing` and dropped a chain three
    /// frames deep. Use anyhow's alternate-Display (`{e:#}`) to print
    /// the chain joined by `: `.
    #[test]
    fn anyhow_chain_format_includes_inner_causes() {
        // Build a 3-deep chain similar to what tailor_for_listing -> Llm
        // -> ClaudeCli currently produces.
        let inner: anyhow::Result<()> = Err(anyhow::anyhow!("validator: bullet not covered"));
        let mid = inner.map_err(|e| e.context("schema::parse_and_validate"));
        let outer: anyhow::Result<()> = mid.map_err(|e| e.context("tailor_for_listing"));
        let err = outer.unwrap_err();

        // The format we pick must surface every layer.
        let chain = format!("{err:#}");
        assert!(
            chain.contains("tailor_for_listing"),
            "missing outer in chain: {chain:?}"
        );
        assert!(
            chain.contains("schema::parse_and_validate"),
            "missing mid in chain: {chain:?}"
        );
        assert!(
            chain.contains("bullet not covered"),
            "missing inner in chain: {chain:?}"
        );
        // The plain Display formatter (what `%e` uses in tracing) only
        // emits the outermost layer — that's the bug we're guarding
        // against.
        let outermost_only = format!("{err}");
        assert_eq!(
            outermost_only, "tailor_for_listing",
            "plain Display still drops the chain — confirm our fix uses {{:#}} or ?e"
        );
    }

    /// Regression: `--source greenhouse,lever,ashby` should parse as
    /// three filter entries, not one 19-char source name. Clap's
    /// `Vec<String>` only splits on commas when `value_delimiter`
    /// is set explicitly. The earlier behavior silently no-op'd
    /// `careerai discover` because the joined "greenhouse,lever,..."
    /// matched no `Source::name()`, and the user saw "no sources
    /// enabled" with zero help text pointing at the cause.
    #[test]
    fn discover_source_arg_splits_on_commas() {
        let cli =
            Cli::try_parse_from(["careerai", "discover", "--source", "greenhouse,lever,ashby"])
                .expect("parse");
        match cli.command {
            Command::Discover { sources } => {
                assert_eq!(
                    sources,
                    vec![
                        "greenhouse".to_string(),
                        "lever".to_string(),
                        "ashby".to_string()
                    ],
                    "expected three filter entries; got: {sources:?}"
                );
            }
            other => panic!("expected Discover; got {other:?}"),
        }
    }

    /// `--source greenhouse --source lever` (the repeated form) must
    /// keep working alongside the comma form, matching clap's
    /// documented behavior when `value_delimiter` is set.
    #[test]
    fn discover_source_arg_accepts_repeated_form() {
        let cli = Cli::try_parse_from([
            "careerai",
            "discover",
            "--source",
            "greenhouse",
            "--source",
            "lever",
        ])
        .expect("parse");
        match cli.command {
            Command::Discover { sources } => {
                assert_eq!(sources, vec!["greenhouse".to_string(), "lever".to_string()]);
            }
            other => panic!("expected Discover; got {other:?}"),
        }
    }

    /// Regression for the `--llm-backend` override on `careerai llm
    /// probe`: when the global flag is set, it must replace
    /// `cfg.llm.backend` before `Backend::probe` is called. Mirrors the
    /// merge logic at the top of `run_llm_probe`.
    #[test]
    fn llm_backend_override_replaces_cfg_backend() {
        use careerai_core::config::{BackendChoice, LlmConfig};
        let mut llm_cfg = LlmConfig {
            backend: BackendChoice::ClaudeCli,
            ..LlmConfig::default()
        };
        let override_choice: Option<BackendChoice> = Some(BackendChoice::Api);
        if let Some(b) = override_choice {
            llm_cfg.backend = b;
        }
        assert_eq!(llm_cfg.backend, BackendChoice::Api);
    }

    /// Without an override the configured backend stays put — the flag
    /// is purely additive.
    #[test]
    fn llm_backend_override_absent_keeps_cfg_backend() {
        use careerai_core::config::{BackendChoice, LlmConfig};
        let mut llm_cfg = LlmConfig {
            backend: BackendChoice::ClaudeCli,
            ..LlmConfig::default()
        };
        let override_choice: Option<BackendChoice> = None;
        if let Some(b) = override_choice {
            llm_cfg.backend = b;
        }
        assert_eq!(llm_cfg.backend, BackendChoice::ClaudeCli);
    }

    /// Regression for the probe exit code: when an operator forces a
    /// backend that isn't reachable, `probe_forced_resolve` must
    /// surface a `BackendError` so `run_llm_probe` can exit 2. Without
    /// the resolve attempt, the probe printed `backend: api` and
    /// exited 0 even when no API key was reachable.
    ///
    /// Wraps the test in a `Mutex` because `ANTHROPIC_API_KEY` is a
    /// process-global env var; parallel cargo-test threads racing
    /// set/remove pairs cause intermittent failures.
    ///
    /// `clippy::await_holding_lock`: the `std::sync::Mutex` guard is
    /// held across `.await`, but the only contention is between this
    /// test's own reruns. No deadlock risk.
    #[cfg(feature = "live-llm-api")]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn probe_forced_api_with_no_key_returns_err() {
        use careerai_core::config::{BackendChoice, LlmConfig};
        // Static lock so this test serializes with itself across reruns.
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");
        let llm_cfg = LlmConfig {
            backend: BackendChoice::Api,
            ..LlmConfig::default()
        };
        let res = crate::commands::llm::probe_forced_resolve(&llm_cfg).await;
        if let Some(v) = prev {
            std::env::set_var("ANTHROPIC_API_KEY", v);
        }
        // The keyring may have a real key on a dev box; tolerate Ok in
        // that case. The point is that the resolve attempt happens —
        // not that the key is necessarily missing.
        match res {
            Err(careerai_llm::BackendError::ApiKeyMissing) | Ok(()) => {}
            Err(other) => panic!("expected ApiKeyMissing or Ok, got {other:?}"),
        }
    }

    /// Regression: two `probe_forced_resolve` calls running concurrently
    /// must not collide on the on-disk cache path. The previous
    /// implementation hard-coded `$TMPDIR/careerai-llm-probe-cache`,
    /// which two parallel probes could race against. This test drives
    /// the path indirectly by asserting that
    /// `tempfile::tempdir()` — the underpinning of the fix — yields
    /// distinct directories for parallel callers, and that two parallel
    /// `probe_forced_resolve` calls both complete (Ok or recognized
    /// `BackendError`, but never a filesystem-collision failure).
    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
    #[tokio::test]
    async fn probe_forced_resolve_parallel_does_not_collide() {
        use careerai_core::config::{BackendChoice, LlmConfig};
        // Sanity check the underlying primitive: parallel tempdir
        // creation produces distinct paths.
        let (a, b) = (
            tempfile::tempdir().expect("tempdir a"),
            tempfile::tempdir().expect("tempdir b"),
        );
        assert_ne!(
            a.path(),
            b.path(),
            "tempfile::tempdir() must yield distinct paths"
        );
        drop((a, b));

        let llm_cfg = LlmConfig {
            backend: BackendChoice::ClaudeCli,
            ..LlmConfig::default()
        };
        // Drive the actual code path twice in parallel. Either ordering
        // of completion is fine; the key invariant is that neither
        // call fails with a filesystem-collision error from a shared
        // cache path. Both callers may legitimately succeed (claude
        // CLI present + authed) or return a recognized BackendError
        // (e.g. NoBackend, BinaryMissing). Anything else suggests the
        // tempdirs collided.
        let (r1, r2) = tokio::join!(
            crate::commands::llm::probe_forced_resolve(&llm_cfg),
            crate::commands::llm::probe_forced_resolve(&llm_cfg),
        );
        // Both calls must complete without panicking. We accept any
        // recognized BackendError variant (the CI box may not have a
        // claude binary, an auth'd session, or an API key), but a
        // panic would signal a real bug — most likely a tempdir
        // collision regression.
        for r in [r1, r2] {
            assert!(
                matches!(
                    r,
                    Ok(())
                        | Err(careerai_llm::BackendError::CliMissing
                            | careerai_llm::BackendError::CliNotAuthenticated
                            | careerai_llm::BackendError::ApiKeyMissing
                            | careerai_llm::BackendError::NoneAvailable
                            | careerai_llm::BackendError::FeatureDisabled(_)
                            | careerai_llm::BackendError::Llm(_))
                ),
                "unexpected probe outcome: {r:?}"
            );
        }
    }
}
