#![allow(clippy::expect_used)]

use std::fs;
use std::path::Path;

const RUN_SCRIPT: &str = include_str!("../../../deploy/runit/careerai-dashboard/run");
const LOG_SCRIPT: &str = include_str!("../../../deploy/runit/careerai-dashboard/log/run");

#[cfg(unix)]
fn assert_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)
        .expect("service script metadata")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "{} must be executable", path.display());
}

#[test]
fn dashboard_runit_run_script_starts_loopback_dashboard() {
    assert!(RUN_SCRIPT.starts_with("#!/usr/bin/env bash\n"));
    assert!(RUN_SCRIPT.contains("set -euo pipefail"));
    assert!(RUN_SCRIPT.contains("CAREERAI_ROOT"));
    assert!(RUN_SCRIPT.contains("status serve"));
    assert!(RUN_SCRIPT.contains("CAREERAI_DASHBOARD_PORT"));
    assert!(RUN_SCRIPT.contains("chpst -u"));
    assert!(!RUN_SCRIPT.contains("CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK=1"));

    let script_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/runit/careerai-dashboard/run");
    assert_executable(&script_path);
}

#[test]
fn dashboard_runit_log_script_uses_svlogd() {
    assert!(LOG_SCRIPT.starts_with("#!/usr/bin/env bash\n"));
    assert!(LOG_SCRIPT.contains("set -euo pipefail"));
    assert!(LOG_SCRIPT.contains("svlogd"));
    assert!(LOG_SCRIPT.contains("chpst -u"));

    let log_script_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/runit/careerai-dashboard/log/run");
    assert_executable(&log_script_path);
}
