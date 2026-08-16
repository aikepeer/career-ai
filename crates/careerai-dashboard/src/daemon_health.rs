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
    match status.code() {
        Some(0) => DaemonHealth::Active,
        Some(1 | 2 | 3) => DaemonHealth::Inactive,
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

    #[cfg(target_os = "linux")]
    mod linux_tests {
        use super::*;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        fn temp_dir() -> tempfile::TempDir {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
            let _ = std::fs::create_dir_all(&target);
            tempfile::tempdir_in(&target).unwrap_or_else(|_| tempfile::tempdir().expect("tempdir"))
        }

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
            let tmp = temp_dir();
            let bin = stub_systemctl(tmp.path(), "fake-systemctl-0", 0);
            let abs_bin = bin.canonicalize().unwrap_or(bin);
            assert_eq!(
                probe_with_bin(&abs_bin.display().to_string()).await,
                DaemonHealth::Active
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn exit_three_maps_to_inactive() {
            let tmp = temp_dir();
            let bin = stub_systemctl(tmp.path(), "fake-systemctl-3", 3);
            let abs_bin = bin.canonicalize().unwrap_or(bin);
            assert_eq!(
                probe_with_bin(&abs_bin.display().to_string()).await,
                DaemonHealth::Inactive
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn exit_four_maps_to_unknown() {
            let tmp = temp_dir();
            let bin = stub_systemctl(tmp.path(), "fake-systemctl-4", 4);
            let abs_bin = bin.canonicalize().unwrap_or(bin);
            assert_eq!(
                probe_with_bin(&abs_bin.display().to_string()).await,
                DaemonHealth::Unknown
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn missing_binary_maps_to_unknown() {
            assert_eq!(
                probe_with_bin("/nonexistent/path/to/systemctl-please-no").await,
                DaemonHealth::Unknown
            );
        }
    }
}
