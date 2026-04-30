//! `careerai inspect` — show an application's row, state history,
//! and rendered artifacts.

use std::path::Path;

use anyhow::Result;

use careerai_pipeline as pipeline;

pub async fn run(cwd: &Path, application_id: &str) -> Result<()> {
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
            tracing::error!(error = %format_args!("{e:#}"), "inspect failed");
            std::process::exit(super::apply::map_apply_error_to_exit_code(&e));
        }
    }
}
