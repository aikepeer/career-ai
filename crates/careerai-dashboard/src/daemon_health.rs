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
    probe_with_bin("systemctl").await
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
    match status.code() {
        Some(0) => DaemonHealth::Active,
        // 3 = inactive/dead.
        Some(3) => DaemonHealth::Inactive,
        // 4 = no-such-unit, anything else = unmodelled exit code.
        _ => DaemonHealth::Unknown,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
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

    #[cfg(target_os = "linux")]
    mod linux_tests {
        use super::*;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        /// Write a shell script in `dir` that exits with `code`, return
        /// the path. Used to simulate `systemctl --user is-active <unit>`
        /// outcomes deterministically.
        fn stub_systemctl(dir: &std::path::Path, name: &str, code: i32) -> std::path::PathBuf {
            let path = dir.join(name);
            let mut f = std::fs::File::create(&path).expect("create stub");
            writeln!(f, "#!/bin/sh\nexit {code}").expect("write stub");
            let mut perms = std::fs::metadata(&path).expect("stat").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("chmod");
            path
        }

        #[tokio::test(flavor = "current_thread")]
        async fn exit_zero_maps_to_active() {
            let tmp = tempfile::tempdir().expect("tempdir");
            let bin = stub_systemctl(tmp.path(), "fake-systemctl-0", 0);
            assert_eq!(
                probe_with_bin(&bin.display().to_string()).await,
                DaemonHealth::Active
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn exit_three_maps_to_inactive() {
            let tmp = tempfile::tempdir().expect("tempdir");
            let bin = stub_systemctl(tmp.path(), "fake-systemctl-3", 3);
            assert_eq!(
                probe_with_bin(&bin.display().to_string()).await,
                DaemonHealth::Inactive
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn exit_four_maps_to_unknown() {
            let tmp = tempfile::tempdir().expect("tempdir");
            let bin = stub_systemctl(tmp.path(), "fake-systemctl-4", 4);
            assert_eq!(
                probe_with_bin(&bin.display().to_string()).await,
                DaemonHealth::Unknown
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn missing_binary_maps_to_unknown() {
            // No file at this path; spawn must fail and we collapse to Unknown.
            assert_eq!(
                probe_with_bin("/nonexistent/path/to/systemctl-please-no").await,
                DaemonHealth::Unknown
            );
        }
    }
}
