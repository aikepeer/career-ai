//! Request types, validation errors, and whitelist parsing for the CLI
//! dispatcher. Pure data + decision logic — no subprocess or HTTP.

use serde::Deserialize;
use std::ffi::OsString;

#[derive(Debug, Deserialize)]
pub struct CliRunRequest {
    pub command: String,
    #[serde(default)]
    pub args: CliRunArgs,
}

#[derive(Debug, Default, Deserialize)]
pub struct CliRunArgs {
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    #[serde(default)]
    pub tune: Option<bool>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub listing_id: Option<String>,
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub all: Option<bool>,
    #[serde(default)]
    pub auto_submit: Option<bool>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub force: Option<bool>,
    #[serde(default)]
    pub apply: Option<bool>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub from_state: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliRunValidationError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available from the dashboard")]
    DisallowedCommand(String),
    #[error("invalid id `{0}`: must be non-empty and contain only [A-Za-z0-9_-]")]
    InvalidId(String),
    #[error("unknown source `{0}`")]
    UnknownSource(String),
    #[error("missing required argument `{0}` for command `{1}`")]
    MissingArgument(&'static str, &'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WhitelistedCommand {
    Discover,
    Match,
    Run,
    ShortlistShow,
    Tailor,
    Render,
    Apply,
    Applied,
    Inspect,
    Review,
    Retry,
    Rollback,
    ConfigGenerate,
    SourcesSync,
    SourcesDiscoverWeb,
    Digest,
    LlmProbe,
    McpProbe,
    NotifyTest,
    ProfileShow,
    ProfileValidate,
}

pub(super) fn parse_command(raw: &str) -> Result<WhitelistedCommand, CliRunValidationError> {
    let normalized = raw.trim().to_ascii_lowercase();
    let cmd = match normalized.as_str() {
        "discover" => WhitelistedCommand::Discover,
        "match" => WhitelistedCommand::Match,
        "run" => WhitelistedCommand::Run,
        "shortlist show" => WhitelistedCommand::ShortlistShow,
        "tailor" => WhitelistedCommand::Tailor,
        "render" => WhitelistedCommand::Render,
        "apply" => WhitelistedCommand::Apply,
        "applied" => WhitelistedCommand::Applied,
        "inspect" => WhitelistedCommand::Inspect,
        "review" => WhitelistedCommand::Review,
        "retry" => WhitelistedCommand::Retry,
        "rollback" => WhitelistedCommand::Rollback,
        "config generate" => WhitelistedCommand::ConfigGenerate,
        "sources sync" => WhitelistedCommand::SourcesSync,
        "sources discover-web" => WhitelistedCommand::SourcesDiscoverWeb,
        "digest" => WhitelistedCommand::Digest,
        "llm probe" => WhitelistedCommand::LlmProbe,
        "mcp probe" => WhitelistedCommand::McpProbe,
        "notify test" => WhitelistedCommand::NotifyTest,
        "profile show" => WhitelistedCommand::ProfileShow,
        "profile validate" => WhitelistedCommand::ProfileValidate,
        "init" | "profile import" | "daemon" | "status serve" | "status" | "service"
        | "service install" | "service status" | "service uninstall" | "cookies refresh" => {
            return Err(CliRunValidationError::DisallowedCommand(raw.to_string()));
        }
        other => return Err(CliRunValidationError::UnknownCommand(other.to_string())),
    };
    Ok(cmd)
}

pub(super) fn os(s: &str) -> OsString {
    OsString::from(s)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn validate_id(value: &str) -> Result<(), CliRunValidationError> {
    if valid_id(value) {
        Ok(())
    } else {
        Err(CliRunValidationError::InvalidId(value.to_string()))
    }
}

pub(super) fn require_id<'a>(
    value: Option<&'a String>,
    field: &'static str,
    command: &'static str,
) -> Result<&'a str, CliRunValidationError> {
    let value = value.ok_or(CliRunValidationError::MissingArgument(field, command))?;
    validate_id(value)?;
    Ok(value.as_str())
}
