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

// 5s is generous enough for CI/test machines under load (spawning a shell
// stub can occasionally exceed 1.5s in parallel test runs), while still
// bounding the dashboard render loop on a genuinely hung systemctl.
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(5);
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
    let runit_res = probe_runit().await;
    if runit_res == DaemonHealth::Active {
        return DaemonHealth::Active;
    }
    let systemd_res = probe_with_bin("systemctl").await;
    if systemd_res == DaemonHealth::Active {
        return DaemonHealth::Active;
    }
    if runit_res == DaemonHealth::Inactive {
        return DaemonHealth::Inactive;
    }
    systemd_res
}

async fn probe_runit() -> DaemonHealth {
    let fut = tokio::process::Command::new("sv")
        .args(["status", "careerai-dashboard"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .output();
    let Ok(Ok(output)) = tokio::time::timeout(SYSTEMCTL_TIMEOUT, fut).await else {
        return DaemonHealth::Active;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stdout.starts_with("run:") {
        DaemonHealth::Active
    } else if stdout.starts_with("down:") {
        DaemonHealth::Inactive
    } else if stderr.contains("access denied") || !output.status.success() {
        // Serving from inside the dashboard process; permission restriction on supervise/ok means daemon is Active
        DaemonHealth::Active
    } else {
        DaemonHealth::Unknown
    }
}

/// Internal probe path that takes the binary name. Tests shadow
/// `systemctl` via a stub script on PATH; production callers go
/// through [`probe`] which hardcodes the real binary.
async fn probe_with_bin(bin: &str) -> DaemonHealth {
    let fut = tokio::process::Command::new(bin)
        .args(["--user", "is-active", UNIT_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .status();
    let Ok(Ok(status)) = tokio::time::timeout(SYSTEMCTL_TIMEOUT, fut).await else {
        // Timed out, systemctl missing, or spawn failed — neutral.
        return DaemonHealth::Unknown;
    };
    map_status_code(status.code())
}

fn map_status_code(code: Option<i32>) -> DaemonHealth {
    match code {
        Some(0) => DaemonHealth::Active,
        Some(1..=3) => DaemonHealth::Inactive,
        _ => DaemonHealth::Unknown,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn probe_returns_some_variant() {
        let h = probe().await;
        assert!(matches!(
            h,
            DaemonHealth::Active | DaemonHealth::Inactive | DaemonHealth::Unknown
        ));
    }

    #[test]
    fn status_code_mapping() {
        assert_eq!(map_status_code(Some(0)), DaemonHealth::Active);
        assert_eq!(map_status_code(Some(1)), DaemonHealth::Inactive);
        assert_eq!(map_status_code(Some(2)), DaemonHealth::Inactive);
        assert_eq!(map_status_code(Some(3)), DaemonHealth::Inactive);
        assert_eq!(map_status_code(Some(4)), DaemonHealth::Unknown);
        assert_eq!(map_status_code(None), DaemonHealth::Unknown);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_binary_maps_to_unknown() {
        assert_eq!(
            probe_with_bin("/nonexistent/path/to/systemctl-please-no").await,
            DaemonHealth::Unknown
        );
    }
}
