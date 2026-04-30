//! Locate and validate the `claude` binary on the host.
//!
//! Resolution order:
//! 1. `CAREERAI_CLAUDE_BIN` env var (if set and non-empty)
//! 2. `which("claude")` on `PATH`
//!
//! Both paths are validated for existence and executability.

use std::path::PathBuf;

use super::error::ClaudeCliError;

pub(crate) fn locate_claude_binary() -> std::result::Result<PathBuf, ClaudeCliError> {
    if let Ok(p) = std::env::var("CAREERAI_CLAUDE_BIN") {
        if !p.is_empty() {
            let path = PathBuf::from(&p);
            return validate_claude_binary(path);
        }
    }
    let resolved = which::which("claude").map_err(|_| ClaudeCliError::NotInstalled)?;
    validate_claude_binary(resolved)
}

/// Verify a candidate `claude` binary path exists and is executable. The
/// `CAREERAI_CLAUDE_BIN` env override is owner-controlled by definition;
/// we still sanity-check the path so a typo or stale value surfaces as a
/// clear `BinaryUnusable` error rather than a confusing spawn ENOENT
/// later. Trust assumption: the env var is set by the operator running
/// the binary, never by network input.
fn validate_claude_binary(path: PathBuf) -> std::result::Result<PathBuf, ClaudeCliError> {
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => {
            return Err(ClaudeCliError::BinaryUnusable {
                path: path.display().to_string(),
                reason: format!("stat: {e}"),
            });
        }
    };
    if !meta.is_file() {
        return Err(ClaudeCliError::BinaryUnusable {
            path: path.display().to_string(),
            reason: "not a regular file".into(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o111 == 0 {
            return Err(ClaudeCliError::BinaryUnusable {
                path: path.display().to_string(),
                reason: format!("no executable bit set (mode 0o{mode:o})"),
            });
        }
    }
    Ok(path)
}
