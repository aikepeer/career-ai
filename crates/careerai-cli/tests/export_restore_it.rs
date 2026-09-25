//! F10: Integration test for `careerai export` and `careerai restore`.
//!
//! Scaffolds a temp project, runs `careerai export` to dump the empty
//! workspace to JSON, then runs `careerai restore --yes` to load it
//! back. Asserts both commands exit zero and the export file is valid
//! JSON with the expected top-level keys.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

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
fn export_creates_valid_json_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    let output = Command::new(env!("CARGO_BIN_EXE_careerai"))
        .args(["export", "--output", "snapshot.json"])
        .env("CAREERAI_ROOT", tmp.path())
        .current_dir(tmp.path())
        .output()
        .expect("spawn careerai export");

    assert!(
        output.status.success(),
        "careerai export failed with stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let snapshot_path = tmp.path().join("snapshot.json");
    assert!(snapshot_path.exists(), "snapshot.json was not created");

    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap())
            .expect("snapshot is valid JSON");
    assert_eq!(json["format_version"], 1, "format_version must be 1");
    assert!(
        json["exported_at"].is_string(),
        "exported_at must be a string"
    );
    assert!(
        json["tables"]["listings"].is_array(),
        "tables.listings must be an array"
    );
}

#[test]
fn restore_loads_exported_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    // Export first.
    let export_out = Command::new(env!("CARGO_BIN_EXE_careerai"))
        .args(["export", "--output", "snapshot.json"])
        .env("CAREERAI_ROOT", tmp.path())
        .current_dir(tmp.path())
        .output()
        .expect("spawn careerai export");
    assert!(
        export_out.status.success(),
        "export failed:\n{}",
        String::from_utf8_lossy(&export_out.stderr),
    );

    // Restore requires --yes.
    let restore_out = Command::new(env!("CARGO_BIN_EXE_careerai"))
        .args(["restore", "--yes", "snapshot.json"])
        .env("CAREERAI_ROOT", tmp.path())
        .current_dir(tmp.path())
        .output()
        .expect("spawn careerai restore");

    assert!(
        restore_out.status.success(),
        "careerai restore failed with stderr:\n{}",
        String::from_utf8_lossy(&restore_out.stderr),
    );

    let stdout = String::from_utf8_lossy(&restore_out.stdout);
    assert!(
        stdout.contains("restored"),
        "restore output must mention 'restored': {stdout}",
    );
}

#[test]
fn restore_refuses_without_yes_flag() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());

    // Create a minimal snapshot.
    fs::write(
        tmp.path().join("snapshot.json"),
        r#"{"format_version":1,"exported_at":"2026-01-01T00:00:00Z","tables":{"listings":[],"applications":[],"application_payloads":[],"events":[],"outcomes":[],"follow_ups":[],"match_reasons":[]}}"#,
    )
    .unwrap();

    let restore_out = Command::new(env!("CARGO_BIN_EXE_careerai"))
        .args(["restore", "snapshot.json"])
        .env("CAREERAI_ROOT", tmp.path())
        .current_dir(tmp.path())
        .output()
        .expect("spawn careerai restore");

    assert!(
        !restore_out.status.success(),
        "restore must fail without --yes: {}",
        String::from_utf8_lossy(&restore_out.stdout),
    );
}
