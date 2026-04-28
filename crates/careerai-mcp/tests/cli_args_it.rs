//! `careerai-mcp --version` / `--help` must exit immediately rather than
//! launch the stdio MCP server (which would block waiting for JSON-RPC
//! frames on stdin and look like a hang to the user).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin_path() -> std::path::PathBuf {
    // Cargo sets CARGO_BIN_EXE_<name> for integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_careerai-mcp"))
}

fn run_with_timeout(args: &[&str], timeout: Duration) -> (i32, String, String) {
    let mut child = Command::new(bin_path())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn careerai-mcp");

    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            let out = child.wait_with_output().expect("wait_with_output");
            return (
                status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            );
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "careerai-mcp {args:?} did not exit within {timeout:?} (looks like the binary launched the stdio server instead of handling the flag)"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn version_flag_prints_and_exits() {
    let (code, stdout, _stderr) = run_with_timeout(&["--version"], Duration::from_secs(2));
    assert_eq!(code, 0, "--version should exit 0");
    assert!(
        stdout.to_lowercase().contains("careerai-mcp"),
        "stdout should mention the binary name; got: {stdout:?}"
    );
    assert!(
        stdout.chars().any(|c| c.is_ascii_digit()),
        "stdout should contain a version number; got: {stdout:?}"
    );
}

#[test]
fn help_flag_prints_and_exits() {
    let (code, stdout, _stderr) = run_with_timeout(&["--help"], Duration::from_secs(2));
    assert_eq!(code, 0, "--help should exit 0");
    assert!(
        stdout.to_lowercase().contains("usage"),
        "stdout should contain a Usage section; got: {stdout:?}"
    );
}
