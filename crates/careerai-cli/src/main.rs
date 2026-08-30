//! `careerai` CLI entry point.
//!
//! Subcommands dispatch into the workspace crates. `init`, `profile import`,
//! `profile show`, `profile validate`, and `--help` are wired end-to-end at
//! M1; the rest are stubs until M2+.
//!
//! Clap structs and tracing init live in `cli_def.rs`; sub-command enums
//! live in `commands/mod.rs`. This file keeps only the dispatcher to stay
//! under the project's 300-LOC cap.

mod cli_def;
mod commands;
mod cookies;
mod digest;
mod review;
mod salary;
mod service;
mod sources_sync;
mod status;

#[cfg(test)]
mod main_tests;

use careerai_pipeline as pipeline;

use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;

use careerai_core::config::CoreConfig;
use cli_def::{init_tracing, Cli, Command};
use commands::{
    CookiesCommand, LlmCommand, McpCommand, NotifyCommand, ProfileCommand, ServiceCommand,
    ShortlistCommand, SourcesCommand, StatusCommand,
};

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log.as_deref());

    // App root: `CAREERAI_ROOT` override → CWD → home-fallback to the
    // project workspace (runit services start in $HOME). Every
    // subcommand reads config/, data/, and profile/ from here.
    let cwd = careerai_core::paths::resolve_root_env();

    // Parse the global `--llm-backend` flag once so subcommands can
    // forward it down without re-parsing.
    let backend_override: Option<careerai_core::config::BackendChoice> =
        match cli.llm_backend.as_deref() {
            Some(s) => Some(s.parse().map_err(|e: String| anyhow::anyhow!(e))?),
            None => None,
        };

    match cli.command {
        Command::Config { command } => match command {
            commands::ConfigSubcommand::Generate { force } => {
                commands::config_cmd::run_generate(force)?;
            }
        },
        Command::Init { force } => {
            careerai_core::init::scaffold(&cwd, force)?;
        }
        Command::Profile { command } => commands::profile::run(command, backend_override)?,
        Command::Discover { sources } => commands::discover::run(&cwd, &sources).await?,
        Command::Match {
            tune,
            rematch_shortlisted,
        } => commands::match_::run(&cwd, tune, rematch_shortlisted).await?,
        Command::Run { auto_submit } => {
            let cfg = load_cfg(&cwd)?;
            commands::run::run(&cwd, &cfg, auto_submit).await?;
        }
        Command::Shortlist { command } => match command {
            ShortlistCommand::Show { limit } => commands::shortlist::run_show(&cwd, limit).await?,
        },
        Command::Tailor {
            listing_id,
            all,
            limit,
        } => {
            let mut cfg = load_cfg(&cwd)?;
            if let Some(b) = backend_override {
                cfg.llm.backend = b;
            }
            if all || limit.is_some() {
                let outcomes = pipeline::tailor_all(&cwd, &cfg, limit).await?;
                println!("tailored: {} shortlisted listings", outcomes.len());
                println!("run `careerai render --all` to emit DOCX/PDF artifacts");
            } else if let Some(id) = listing_id {
                match pipeline::tailor_one(&cwd, &cfg, &id).await {
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
            } else {
                anyhow::bail!("provide listing_id or pass --all or --limit <n>");
            }
        }
        Command::Render {
            application_id,
            all,
        } => {
            let cfg = load_cfg(&cwd)?;
            if all {
                let outcomes = pipeline::render_all(&cwd, &cfg).await?;
                println!("rendered: {} tailored applications", outcomes.len());
            } else if let Some(id) = application_id {
                match pipeline::render_one(&cwd, &cfg, &id).await {
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
            } else {
                anyhow::bail!("provide application_id or pass --all");
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
            SourcesCommand::DiscoverWeb { apply } => {
                let agent = careerai_sources::WebSearchDiscoveryAgent::default();
                let portals = agent.discover_portals();
                println!("🌐 Web Search Discovery Agent — Discovered Job & Freelance Portals:");
                println!(
                    "{:<22} {:<15} {:<32} {:<30}",
                    "NAME", "CATEGORY", "BASE URL", "DESCRIPTION"
                );
                println!("{}", "-".repeat(100));
                for p in &portals {
                    println!(
                        "{:<22} {:<15} {:<32} {:<30}",
                        p.name, p.category, p.base_url, p.description
                    );
                }

                if apply {
                    let config_path = cwd.join("config").join("local.yaml");
                    let added = agent.apply_to_config(&config_path, &portals)?;
                    println!(
                        "\n✅ Successfully updated {} with {} newly discovered portals!",
                        config_path.display(),
                        added
                    );
                } else {
                    println!("\n💡 Run `careerai sources discover-web --apply` to append these portals directly into config/local.yaml.");
                }
            }
        },
        Command::Salary {
            company,
            city,
            json,
            list_all,
            validate,
        } => {
            salary::run(
                &cwd,
                &salary::SalaryArgs {
                    company,
                    city,
                    json,
                    list_all,
                    validate,
                },
            )?;
        }
        Command::Inspect { application_id } => {
            commands::inspect::run(&cwd, &application_id).await?;
        }
        Command::Retry { application_id } => {
            let cfg = load_cfg(&cwd)?;
            commands::retry::run(&cwd, &cfg, &application_id).await?;
        }
        Command::Rollback {
            id,
            to,
            all,
            from_state,
        } => {
            if all {
                let from = from_state.as_deref().unwrap_or("rendered");
                let outcomes = pipeline::rollback_all(&cwd, from, to.as_deref()).await?;
                println!(
                    "rolled back {} items from `{}` to `{}`",
                    outcomes.len(),
                    from,
                    to.as_deref().unwrap_or("previous")
                );
            } else if let Some(target_id) = id {
                let outcome = pipeline::rollback_one(&cwd, &target_id, to.as_deref()).await?;
                println!(
                    "rolled back {} from `{}` to `{}`",
                    outcome.id, outcome.from_state, outcome.to_state
                );
            } else {
                anyhow::bail!("provide id or pass --all");
            }
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

fn load_cfg(cwd: &Path) -> Result<CoreConfig> {
    CoreConfig::load(cwd).context("load config")
}
