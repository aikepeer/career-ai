//! Probe whether `careerai daemon` is currently running.
//!
//! Strategy: shell out to `systemctl --user is-active careerai`. The
//! command is cheap (~10 ms on a warm system) and exits with code 0
//! when active, non-zero otherwise. Anything else — `systemctl`
//! missing, no unit file, the user installed without our `service`
//! subcommand — collapses into `Unknown`, which the template renders
//! neutrally rather than misleadingly green/red.
//!
//! Linux-only in practice. Other platforms always return `Unknown`.

use serde::Serialize;
use std::time::Duration;

const SYSTEMCTL_TIMEOUT: Duration = Duration::from_millis(1500);
const UNIT_NAME: &str = "careerai";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DaemonHealth {
    Active,
    Inactive,
    Unknown,
}

pub async fn probe() -> DaemonHealth {
    if !cfg!(target_os = "linux") {
        return DaemonHealth::Unknown;
    }
    let fut = tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", UNIT_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .status();
    let Ok(Ok(status)) = tokio::time::timeout(SYSTEMCTL_TIMEOUT, fut).await else {
        // Timed out, systemctl missing, or spawn failed — neutral.
        return DaemonHealth::Unknown;
    };
    match status.code() {
        Some(0) => DaemonHealth::Active,
        // 3 = inactive/dead.
        Some(3) => DaemonHealth::Inactive,
        // 4 = no-such-unit, anything else = unmodelled exit code.
        _ => DaemonHealth::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn probe_returns_some_variant() {
        // We don't control whether systemctl exists on the runner. The
        // public contract is just "returns one of the three variants
        // and never panics".
        let h = probe().await;
        assert!(matches!(
            h,
            DaemonHealth::Active | DaemonHealth::Inactive | DaemonHealth::Unknown
        ));
    }
}
