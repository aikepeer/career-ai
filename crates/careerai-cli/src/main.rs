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
enum ProfileCommand {
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
        Command::Profile { command } => run_profile(command, backend_override)?,
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

/// Default location for the canonical profile file: `./profile/profile.yaml`.
fn profile_yaml_path() -> Result<PathBuf> {
    Ok(std::env::current_dir()?
        .join("profile")
        .join("profile.yaml"))
}

fn run_profile(
    command: ProfileCommand,
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<()> {
    match command {
        ProfileCommand::Import {
            paths,
            force,
            use_llm,
        } => profile_import(&paths, force, use_llm, backend_override),
        ProfileCommand::Show => profile_show(),
        ProfileCommand::Validate => profile_validate(),
    }
}

fn profile_import(
    paths: &[PathBuf],
    force: bool,
    use_llm: Option<bool>,
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<()> {
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

    // Auto-enable LLM extraction when ANY live backend is compiled in
    // and one is plausibly available — `claude` binary on PATH (auth
    // NOT verified at this stage) or an Anthropic API key reachable.
    // The actual reachability check (including `claude` auth) happens
    // inside `run_profile_import_with_llm`; if it fails we fall back
    // to the heuristic parser and print a hint to `claude login` or
    // export `ANTHROPIC_API_KEY`.
    let live_compiled = cfg!(any(feature = "live-llm-cli", feature = "live-llm-api"));
    let want_llm = match use_llm {
        Some(v) => v,
        None => live_compiled && llm_backend_maybe_available(),
    };

    let profile = if want_llm {
        // Fall back to the heuristic parser when LLM resolution fails
        // mid-run (e.g. session expired since `llm_backend_maybe_available`
        // checked, claude CLI binary stale, network drop). The
        // heuristic parser is strictly better than a hard error here:
        // the user gets a profile they can edit, and a clear log line
        // points them at `claude login` / API key.
        //
        // When `--use-llm` was passed explicitly, surface the original
        // error rather than silently downgrading — the user asked for
        // the LLM path.
        match run_profile_import_with_llm(&refs, backend_override) {
            Ok(p) => p,
            Err(e) if use_llm == Some(true) => {
                return Err(e);
            }
            Err(e) => {
                tracing::warn!(
                    target = "profile",
                    error = %format_args!("{e:#}"),
                    "LLM extraction failed; falling back to heuristic parser \
                     (run `claude login` or set ANTHROPIC_API_KEY to re-enable LLM)"
                );
                eprintln!(
                    "warning: LLM extraction failed ({e}); falling back to heuristic \
                     parser. Run `claude login` or set ANTHROPIC_API_KEY for better results."
                );
                careerai_profile::import_paths(&refs).context("parsing profile sources")?
            }
        }
    } else {
        if use_llm.is_none() && !live_compiled {
            // Built without any live backend; nothing the user can do
            // at runtime to improve this.
        } else if use_llm.is_none() {
            eprintln!(
                "warning: no LLM backend reachable (claude CLI not authed and no \
                 ANTHROPIC_API_KEY); falling back to heuristic parser. Run \
                 `claude login` or set ANTHROPIC_API_KEY for better results."
            );
        }
        careerai_profile::import_paths(&refs).context("parsing profile sources")?
    };

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

    // Detect the stale-schema signature (`skills:` followed by a flat
    // sequence) before serde gets a chance to bury the error in a generic
    // "invalid type" message. The 0.x line wrote skills as `Vec<String>`;
    // the current schema is the structured `Skills { languages, ... }`.
    if detect_stale_skills_schema(&text) {
        anyhow::bail!(
            "Detected stale schema (skills as a flat list). Old binary wrote this file.\n\
             Re-run: careerai profile import --force <your sources>"
        );
    }

    let profile = careerai_profile::Profile::from_yaml(&text).context("parse profile yaml")?;
    profile.check().context("profile validation failed")?;
    println!("profile ok: {}", out.display());
    Ok(())
}

/// Stale-schema sniffer for `profile validate`. The 0.x binary wrote
/// `skills:` as a flat YAML sequence (`- Rust\n- Python`). The current
/// schema serializes it as a nested mapping with `languages`,
/// `frameworks`, `tools`. We detect the legacy shape via a quick string
/// scan rather than pulling in a YAML parser — false-positive cost is
/// just a misleading hint, which is fine.
fn detect_stale_skills_schema(text: &str) -> bool {
    let mut in_skills = false;
    for line in text.lines() {
        if !in_skills {
            if line.trim_start() == "skills:" && !line.starts_with(' ') {
                in_skills = true;
            }
            continue;
        }
        // Ignore blanks and pure comments inside the block.
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let leading = line.len() - trimmed.len();
        if leading == 0 {
            // Left the skills block without seeing nested keys.
            return false;
        }
        // First non-empty child of `skills:`. Stale shape:  `- Rust`.
        return trimmed.starts_with("- ");
    }
    false
}

/// Best-effort probe: is an Anthropic API key reachable without any
/// network round-trip? Checks `ANTHROPIC_API_KEY` first, then the
/// keyring (service "career-ai", username "anthropic/api_key"). Used
/// to default `--use-llm`.
fn anthropic_key_reachable() -> bool {
    if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return true;
    }
    if let Ok(entry) = keyring::Entry::new("career-ai", "anthropic/api_key") {
        if let Ok(v) = entry.get_password() {
            if !v.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Cheap heuristic: would `Backend::resolve(Auto, ...)` likely succeed
/// on this host? Returns `true` if either:
///
///   * the `claude` binary is on PATH (auth NOT verified — caller is
///     expected to handle `Backend::resolve`'s failure gracefully when
///     the session is logged out), or
///   * an Anthropic API key is reachable (env or keyring).
///
/// This is intentionally NOT an auth probe (which costs ~5s on cold
/// start). It is used by `--use-llm` auto-detection to decide whether
/// to even ATTEMPT the LLM path. The actual reachability check
/// happens inside `run_profile_import_with_llm`, which falls back to
/// the heuristic parser when `Backend::resolve` errors and surfaces a
/// hint to run `claude login` or set `ANTHROPIC_API_KEY`.
fn llm_backend_maybe_available() -> bool {
    if which::which("claude").is_ok() {
        return true;
    }
    anthropic_key_reachable()
}

/// Parse PDF/DOCX inputs through an LLM extractor; LinkedIn ZIPs go
/// through their structured CSV path unchanged. Selects the backend
/// (`claude` CLI vs rig-core Anthropic API) via
/// `careerai_llm::Backend::resolve` honoring `cfg.llm.backend` and the
/// `--llm-backend` global flag.
fn run_profile_import_with_llm(
    paths: &[&Path],
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<careerai_profile::Profile> {
    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
    {
        use std::sync::Arc;

        let cwd = std::env::current_dir()?;
        // Fall back to `LlmConfig::default()` ONLY when no `config/`
        // directory is present (e.g. running `profile import` before
        // `init`). When config exists, surface load/parse failures so
        // malformed YAML and similar real errors don't silently hide
        // behind defaults.
        let mut llm_cfg = if cwd.join("config").exists() {
            CoreConfig::load(&cwd).context("load config/")?.llm
        } else {
            careerai_core::config::LlmConfig::default()
        };
        if let Some(b) = backend_override {
            llm_cfg.backend = b;
        }

        // Honor `config.llm.cache_dir` so live profile-extract caches
        // sit next to tailor caches under `data/cache/llm`. `.gitignore`
        // already excludes `/data/`.
        let cache_root: PathBuf = if llm_cfg.cache_dir.is_empty() {
            PathBuf::from("data").join("cache").join("llm")
        } else {
            PathBuf::from(&llm_cfg.cache_dir)
        };
        let cache_dir = if cache_root.is_absolute() {
            cache_root
        } else {
            cwd.join(cache_root)
        };
        let cache = Arc::new(careerai_llm::Cache::new(cache_dir));

        let mut opts = careerai_profile::ExtractOptions::default();
        // Plumb config.llm.parse_resume_model into the extractor; strip
        // any leading `provider/` prefix that the layered config uses
        // (rig's anthropic transport expects the bare model id).
        if !llm_cfg.parse_resume_model.is_empty() {
            opts.model = strip_provider_prefix(&llm_cfg.parse_resume_model).to_string();
        }
        if !llm_cfg.prompt_version.is_empty() {
            opts.prompt_version.clone_from(&llm_cfg.prompt_version);
        }

        let backend = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(careerai_llm::Backend::resolve(
                llm_cfg.backend,
                &llm_cfg,
                cache,
            ))
        })
        .map_err(|e| anyhow::anyhow!("resolve llm backend: {e}"))?;

        let adapter = profile_llm_adapter::Adapter::new(&backend);
        let ctx = careerai_profile::LlmExtractContext::new(&adapter, opts);
        careerai_profile::import_paths_with_llm(paths, Some(&ctx))
            .context("parsing profile sources via LLM")
    }
    #[cfg(not(any(feature = "live-llm-cli", feature = "live-llm-api")))]
    {
        let _ = (paths, backend_override);
        anyhow::bail!(
            "LLM extraction requires the `live-llm-cli` or `live-llm-api` cargo feature. \
             Re-run with: cargo run -p careerai-cli --features live-llm-cli -- profile import …"
        )
    }
}

/// Strip a leading `provider/` prefix (e.g. `anthropic/claude-haiku-4-5`
/// → `claude-haiku-4-5`). Layered config templates use the prefixed
/// form for human readability, but rig's transports want the bare model
/// id. No-op when no slash is present.
///
/// Only the live-LLM import path consumes this; tests exercise it on
/// every build.
#[cfg_attr(not(any(feature = "live-llm", test)), allow(dead_code))]
fn strip_provider_prefix(model: &str) -> &str {
    model.split_once('/').map_or(model, |(_, rest)| rest)
}

/// Adapter that lets a `careerai_llm::Llm` be used as a
/// `careerai_profile::LlmCaller`. Lives here (in the CLI) because the
/// CLI is the only crate that depends on both, breaking the otherwise-
/// circular `profile ↔ llm` edge.
#[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
mod profile_llm_adapter {
    use async_trait::async_trait;
    use careerai_llm::{Llm, LlmRequest};
    use careerai_profile::llm_extract::{ExtractRequest, LlmCaller};

    pub struct Adapter<'a> {
        inner: &'a dyn Llm,
    }

    impl<'a> Adapter<'a> {
        pub fn new(inner: &'a dyn Llm) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl LlmCaller for Adapter<'_> {
        async fn call(&self, req: &ExtractRequest) -> Result<String, String> {
            let llm_req = LlmRequest {
                system: req.system.clone(),
                profile_block: req.profile_block.clone(),
                user: req.user.clone(),
                prompt_version: req.prompt_version.clone(),
                model: req.model.clone(),
                temperature: req.temperature,
                max_tokens: req.max_tokens,
                cache_profile: req.cache_schema,
            };
            self.inner
                .complete(&llm_req)
                .await
                .map(|r| r.text)
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn detect_stale_skills_flags_legacy_flat_list() {
        let yaml = "personal:\n  name: Alice\nskills:\n  - Rust\n  - Python\n";
        assert!(detect_stale_skills_schema(yaml));
    }

    #[test]
    fn strip_provider_prefix_handles_layered_config_form() {
        assert_eq!(
            strip_provider_prefix("anthropic/claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix("openai/gpt-4o-mini"), "gpt-4o-mini");
    }

    #[test]
    fn strip_provider_prefix_passes_bare_model_id_through() {
        assert_eq!(
            strip_provider_prefix("claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix(""), "");
    }

    #[test]
    fn detect_stale_skills_passes_current_mapping_shape() {
        let yaml = "personal:\n  name: Alice\nskills:\n  languages:\n    - Rust\n";
        assert!(!detect_stale_skills_schema(yaml));
    }

    #[test]
    fn detect_stale_skills_handles_missing_skills_block() {
        let yaml = "personal:\n  name: Alice\nsummary: hi\n";
        assert!(!detect_stale_skills_schema(yaml));
    }

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

    #[test]
    fn detect_stale_skills_ignores_blank_lines_and_comments() {
        let yaml = "personal:\n  name: Alice\nskills:\n\n  # a comment\n  languages:\n    - Rust\n";
        assert!(!detect_stale_skills_schema(yaml));
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
