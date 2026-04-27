//! `careerai review` — interactive walkthrough of drafted LinkedIn
//! applications.
//!
//! Daemon-side, LinkedIn applies always end at `Drafted` (the
//! `interactive_only=true` short-circuit in `apply_one`). This is the only
//! code path that turns Drafted → Submitted, by prompting the operator y/N/s
//! per draft and calling `pipeline::confirm_linkedin_submit` on yes.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

/// Walk all drafted LinkedIn applications and prompt [y/N/s(kip remaining)]
/// for each. On `y`, delegates to `pipeline::confirm_linkedin_submit` which
/// overrides `interactive_only` for the single invocation and drives the
/// browser click.
pub async fn run_review(root: &Path, cfg: &CoreConfig) -> Result<()> {
    let drafted = pipeline::list_drafted_linkedin(root, 100)
        .await
        .context("list drafted linkedin applications")?;

    if drafted.is_empty() {
        println!("No drafted LinkedIn applications. Nothing to review.");
        return Ok(());
    }

    println!(
        "{} drafted LinkedIn application(s) ready for review.\n",
        drafted.len()
    );

    let stdin = io::stdin();
    for (idx, app) in drafted.iter().enumerate() {
        println!("[{}/{}] application {}", idx + 1, drafted.len(), app.id);
        // The operator can run `careerai inspect <id>` for full details.
        print!("  Submit? [y/N/s(kip remaining)]: ");
        // Propagate flush errors — a swallowed broken-pipe makes the
        // prompt invisible while read_line still returns Ok(0), turning
        // every remaining draft into a silent "no" with no indication
        // anything went wrong.
        io::stdout().flush().context("flush stdout")?;

        let mut line = String::new();
        let bytes = stdin.read_line(&mut line).context("read stdin")?;
        if bytes == 0 {
            // EOF — stdin closed (piped from /dev/null, broken pipe,
            // unattended cron). Surface the count of unprocessed drafts
            // explicitly rather than silently treating each as skipped.
            println!(
                "(stdin closed — {} draft(s) left in drafted state)",
                drafted.len() - idx
            );
            break;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => match pipeline::confirm_linkedin_submit(root, cfg, &app.id).await {
                Ok(outcome) => println!("    submitted: {:?}", outcome.outcome),
                Err(e) => eprintln!("    failed: {e}"),
            },
            "s" | "skip-all" => {
                println!("Skipping remaining {} application(s).", drafted.len() - idx);
                break;
            }
            _ => println!("    skipped (left in drafted state)"),
        }
    }
    Ok(())
}
