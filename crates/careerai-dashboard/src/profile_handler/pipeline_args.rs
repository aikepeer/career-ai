//! Per-command argv construction for the whitelisted CLI dispatcher.
//!
//! Kept free of subprocess, lock, and HTTP concerns so the whitelist,
//! rejection, and shell-injection rules are directly unit-testable.

use std::ffi::OsString;

use super::pipeline_types::{
    os, parse_command, require_id, CliRunArgs, CliRunRequest, CliRunValidationError,
    WhitelistedCommand,
};
use super::sources::is_known_source;

fn discover_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    let mut out = vec![os("discover")];
    if let Some(sources) = &args.sources {
        if sources.is_empty() {
            return Err(CliRunValidationError::MissingArgument(
                "sources", "discover",
            ));
        }
        for source in sources {
            if !is_known_source(source) {
                return Err(CliRunValidationError::UnknownSource(source.clone()));
            }
        }
        out.push(os("--source"));
        out.push(os(&sources.join(",")));
    }
    Ok(out)
}

fn match_args(args: &CliRunArgs) -> Vec<OsString> {
    let mut out = vec![os("match")];
    if args.tune == Some(true) {
        out.push(os("--tune"));
    }
    out
}

fn run_args(args: &CliRunArgs) -> Vec<OsString> {
    let mut out = vec![os("run")];
    if args.auto_submit == Some(true) {
        out.push(os("--auto-submit"));
    }
    out
}

fn shortlist_show_args(args: &CliRunArgs) -> Vec<OsString> {
    let mut out = vec![os("shortlist"), os("show")];
    if let Some(limit) = args.limit {
        out.push(os("--limit"));
        out.push(os(&limit.to_string()));
    }
    out
}

fn tailor_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    Ok(vec![
        os("tailor"),
        os(require_id(
            args.listing_id.as_ref(),
            "listing_id",
            "tailor",
        )?),
    ])
}

fn render_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    Ok(vec![
        os("render"),
        os(require_id(
            args.application_id.as_ref(),
            "application_id",
            "render",
        )?),
    ])
}

fn apply_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    let mut out = vec![os("apply")];
    if args.application_id.is_some() {
        let id = require_id(args.application_id.as_ref(), "application_id", "apply")?;
        out.push(os(id));
    } else if args.all == Some(true) {
        out.push(os("--all"));
    } else {
        return Err(CliRunValidationError::MissingArgument(
            "application_id or all",
            "apply",
        ));
    }
    if args.auto_submit == Some(true) {
        out.push(os("--auto-submit"));
    }
    if let Some(source) = &args.source {
        if !is_known_source(source) {
            return Err(CliRunValidationError::UnknownSource(source.clone()));
        }
        out.push(os("--source"));
        out.push(os(source));
    }
    Ok(out)
}

fn applied_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    let mut out = vec![os("applied")];
    if let Some(source) = &args.source {
        if !is_known_source(source) {
            return Err(CliRunValidationError::UnknownSource(source.clone()));
        }
        out.push(os("--source"));
        out.push(os(source));
    }
    if let Some(limit) = args.limit {
        out.push(os("--limit"));
        out.push(os(&limit.to_string()));
    }
    Ok(out)
}

fn inspect_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    Ok(vec![
        os("inspect"),
        os(require_id(
            args.application_id.as_ref(),
            "application_id",
            "inspect",
        )?),
    ])
}

fn review_args() -> Vec<OsString> {
    vec![os("review")]
}

fn retry_args(args: &CliRunArgs) -> Result<Vec<OsString>, CliRunValidationError> {
    Ok(vec![
        os("retry"),
        os(require_id(
            args.application_id.as_ref(),
            "application_id",
            "retry",
        )?),
    ])
}

fn config_generate_args(args: &CliRunArgs) -> Vec<OsString> {
    let mut out = vec![os("config"), os("generate")];
    if args.force == Some(true) {
        out.push(os("--force"));
    }
    out
}

fn sources_sync_args(args: &CliRunArgs) -> Vec<OsString> {
    let mut out = vec![os("sources"), os("sync")];
    if args.apply == Some(true) {
        out.push(os("--apply"));
    }
    out
}

fn digest_args(args: &CliRunArgs) -> Vec<OsString> {
    let mut out = vec![os("digest")];
    if let Some(since) = &args.since {
        out.push(os("--since"));
        out.push(os(since));
    }
    out
}

fn llm_probe_args() -> Vec<OsString> {
    vec![os("llm"), os("probe")]
}

fn notify_test_args() -> Vec<OsString> {
    vec![os("notify"), os("test")]
}

/// Build the validated argv (excluding the executable) for a whitelisted
/// command. Pure function: no subprocess, no shell, no string interpolation
/// into a command line — every user value is a separate `OsString` argument.
pub fn build_command_args(req: &CliRunRequest) -> Result<Vec<OsString>, CliRunValidationError> {
    let command = parse_command(&req.command)?;
    let args = &req.args;
    match command {
        WhitelistedCommand::Discover => discover_args(args),
        WhitelistedCommand::Match => Ok(match_args(args)),
        WhitelistedCommand::Run => Ok(run_args(args)),
        WhitelistedCommand::ShortlistShow => Ok(shortlist_show_args(args)),
        WhitelistedCommand::Tailor => tailor_args(args),
        WhitelistedCommand::Render => render_args(args),
        WhitelistedCommand::Apply => apply_args(args),
        WhitelistedCommand::Applied => applied_args(args),
        WhitelistedCommand::Inspect => inspect_args(args),
        WhitelistedCommand::Review => Ok(review_args()),
        WhitelistedCommand::Retry => retry_args(args),
        WhitelistedCommand::ConfigGenerate => Ok(config_generate_args(args)),
        WhitelistedCommand::SourcesSync => Ok(sources_sync_args(args)),
        WhitelistedCommand::Digest => Ok(digest_args(args)),
        WhitelistedCommand::LlmProbe => Ok(llm_probe_args()),
        WhitelistedCommand::NotifyTest => Ok(notify_test_args()),
    }
}

#[cfg(test)]
#[path = "pipeline_args_tests.rs"]
mod tests;
