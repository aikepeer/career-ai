//! End-to-end smoke test for `careerai run` against the real binary.
//!
//! Spawns the actual `careerai` executable in a temp project directory and
//! asserts a zero exit code plus the submitted/drafted/failed summary line.
//! With an empty DB the pipeline still completes each stage (all counts
//! zero), so this proves the dispatch path end-to-end without fixtures.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

/// Scaffold the minimum directory tree `careerai run` expects under `root`.
/// Mirrors the pattern used by `careerai-cli/tests/apply_it.rs`; the profile
/// is the bare-minimum YAML the loader accepts.
fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(
        root.join("profile").join("profile.yaml"),
        "personal:\n  name: \"Test User\"\n  email: \"test@example.com\"\n  phone: \"\"\nsummary: \"\"\nskills:\n  languages: []\n",
    )
    .unwrap();
}

#[test]
fn run_command_exits_zero_and_prints_apply_counts() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let output = Command::new(env!("CARGO_BIN_EXE_careerai"))
        .arg("run")
        .env("CAREERAI_ROOT", tmp.path())
        .current_dir(tmp.path())
        .output()
        .expect("spawn careerai run");

    assert!(
        output.status.success(),
        "careerai run failed with stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("submitted"),
        "missing submitted count in output: {stdout}",
    );
    assert!(
        stdout.contains("drafted"),
        "missing drafted count in output: {stdout}",
    );
    assert!(
        stdout.contains("failed"),
        "missing failed count in output: {stdout}",
    );
}
