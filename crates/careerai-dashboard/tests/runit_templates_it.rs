#![allow(clippy::expect_used)]

#[cfg(unix)]
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

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

// runsv never reads a service's env/ directory, so values an operator puts
// there must be loaded by the scripts themselves. These tests run the real
// scripts against fake chpst/svlogd/careerai binaries that record their calls.

const FAKE_CHPST: &str = r#"#!/usr/bin/env bash
set -euo pipefail
while [ "$#" -gt 0 ]; do
  case "$1" in
    -e) for f in "$2"/*; do [ -f "$f" ] && export "${f##*/}=$(head -n1 "$f")"; done; shift 2 ;;
    -u) printf 'chpst -u %s\n' "$2" >>"$RUNIT_TRACE"; shift 2 ;;
    *) break ;;
  esac
done
exec "$@"
"#;

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().expect("parent dir")).expect("create parent dir");
    fs::write(path, body).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod script");
}

#[cfg(unix)]
fn service_fixture(name: &str) -> tempfile::TempDir {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("careerai-runit-{name}-"))
        .tempdir()
        .expect("create tempdir");
    let root = tmp.path();
    write_executable(&root.join("bin/chpst"), FAKE_CHPST);
    let recorder =
        "#!/usr/bin/env bash\nprintf '%s %s\\n' \"${0##*/}\" \"$*\" >>\"$RUNIT_TRACE\"\n";
    write_executable(&root.join("bin/svlogd"), recorder);
    write_executable(&root.join("bin/careerai"), recorder);
    tmp
}

#[cfg(unix)]
fn run_service_script(root: &Path, script: &Path, cwd: &Path) -> (std::process::Output, String) {
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let trace = root.join("trace");
    let mut cmd = Command::new(script);
    cmd.current_dir(cwd)
        .env("PATH", path)
        .env("RUNIT_TRACE", &trace);
    for var in [
        "CAREERAI_ROOT",
        "CAREERAI_USER",
        "CAREERAI_BIN",
        "CAREERAI_DASHBOARD_PORT",
        "CAREERAI_LOG_USER",
        "CAREERAI_LOG_DIR",
        "CAREERAI_ENVDIR_LOADED",
    ] {
        cmd.env_remove(var);
    }
    let output = cmd.output().expect("run service script");
    (output, fs::read_to_string(trace).unwrap_or_default())
}

#[cfg(unix)]
#[test]
fn dashboard_log_script_reads_service_envdir() {
    let fixture = service_fixture("log");
    let root = fixture.path();
    let service = root.join("careerai-dashboard");
    write_executable(&service.join("log/run"), LOG_SCRIPT);
    fs::create_dir_all(service.join("env")).expect("create envdir");
    fs::write(service.join("env/CAREERAI_LOG_USER"), "svc-logger\n").expect("write envdir value");

    let (output, trace) = run_service_script(root, &service.join("log/run"), &service.join("log"));

    assert!(
        output.status.success(),
        "log/run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(trace, "chpst -u svc-logger\nsvlogd ./main\n");
}

#[cfg(unix)]
#[test]
fn dashboard_run_script_reads_service_envdir() {
    let fixture = service_fixture("run");
    let root = fixture.path();
    let service = root.join("careerai-dashboard");
    write_executable(&service.join("run"), RUN_SCRIPT);
    let app_root = root.join("app");
    fs::create_dir_all(&app_root).expect("create app root");
    fs::create_dir_all(service.join("env")).expect("create envdir");
    for (key, value) in [
        ("CAREERAI_ROOT", app_root.display().to_string()),
        ("CAREERAI_USER", "svc-user".to_owned()),
        (
            "CAREERAI_BIN",
            root.join("bin/careerai").display().to_string(),
        ),
    ] {
        fs::write(service.join("env").join(key), format!("{value}\n")).expect("write envdir value");
    }

    let (output, trace) = run_service_script(root, &service.join("run"), &service);

    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        trace,
        "chpst -u svc-user\ncareerai status serve --port 8787\n"
    );
}
