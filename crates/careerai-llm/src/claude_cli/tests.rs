#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unwrap_in_result)]

use std::path::PathBuf;

use super::*;
use crate::error::LlmError;

use crate::ENV_LOCK;

mod stub_binary_tests;

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

#[test]
fn parse_real_fixture_shape() {
    let body = std::fs::read_to_string(fixture_path("claude_cli_ping.json")).unwrap();
    let parsed: ClaudeCliResult = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed.is_error, Some(false));
    assert_eq!(parsed.result.as_deref(), Some("OK"));
    let u = parsed.usage.expect("usage present");
    assert_eq!(u.input_tokens, 3);
    assert_eq!(u.output_tokens, 4);
}

#[test]
fn classify_auth_expired_payload() {
    let p = ClaudeCliResult {
        is_error: Some(true),
        api_error_status: None,
        result: Some("Not logged in · Please run /login".into()),
        ..Default::default()
    };
    assert!(matches!(
        classify_error_payload(&p),
        ClaudeCliError::AuthExpired
    ));
}

#[test]
fn classify_invalid_api_key() {
    let p = ClaudeCliResult {
        is_error: Some(true),
        api_error_status: Some(401),
        result: Some("Invalid API key · Fix external API key".into()),
        ..Default::default()
    };
    assert!(matches!(
        classify_error_payload(&p),
        ClaudeCliError::AuthExpired
    ));
}

#[test]
fn classify_rate_limit() {
    let p = ClaudeCliResult {
        is_error: Some(true),
        api_error_status: Some(429),
        result: Some("rate limit hit".into()),
        ..Default::default()
    };
    match classify_error_payload(&p) {
        ClaudeCliError::RateLimited { .. } => {}
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[test]
fn classify_unknown_falls_back_to_transport() {
    let p = ClaudeCliResult {
        is_error: Some(true),
        api_error_status: None,
        result: Some("upstream blew up".into()),
        ..Default::default()
    };
    match classify_error_payload(&p) {
        ClaudeCliError::Transport(s) => assert!(s.contains("upstream")),
        other => panic!("expected Transport, got {other:?}"),
    }
}

/// `CAREERAI_CLAUDE_BIN` pointing at a real executable file resolves
/// cleanly and the returned PathBuf matches the override.
#[test]
fn locate_binary_via_env_override() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    let p = locate_claude_binary().unwrap();
    assert_eq!(p, bin);
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
}

/// `CAREERAI_CLAUDE_BIN` pointing at a missing path surfaces
/// `BinaryUnusable` (NOT a vague NotInstalled), so users debugging a
/// stale env var see exactly which path failed.
#[test]
fn locate_binary_rejects_missing_override() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::env::set_var("CAREERAI_CLAUDE_BIN", "/tmp/careerai-does-not-exist");
    let err = locate_claude_binary().unwrap_err();
    match err {
        ClaudeCliError::BinaryUnusable { path, reason } => {
            assert!(path.contains("careerai-does-not-exist"));
            assert!(reason.contains("stat") || reason.contains("not a regular file"));
        }
        other => panic!("expected BinaryUnusable, got {other:?}"),
    }
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
}

/// `CAREERAI_CLAUDE_BIN` pointing at a non-executable file (e.g. a
/// stale text file) surfaces `BinaryUnusable` with an "exec" hint.
#[cfg(unix)]
#[test]
fn locate_binary_rejects_non_executable_override() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("not-exec");
    std::fs::write(&bin, b"plain text").unwrap();
    // Default mode 0o644 — no exec bit.
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    let err = locate_claude_binary().unwrap_err();
    match err {
        ClaudeCliError::BinaryUnusable { reason, .. } => {
            assert!(reason.contains("executable"), "reason was: {reason}");
        }
        other => panic!("expected BinaryUnusable, got {other:?}"),
    }
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
}

#[test]
fn cli_error_maps_to_llm_error() {
    let e: LlmError = ClaudeCliError::AuthExpired.into();
    match e {
        LlmError::Upstream(s) => assert!(s.contains("not authenticated")),
        other => panic!("expected Upstream, got {other:?}"),
    }
    let e: LlmError = ClaudeCliError::Timeout { seconds: 12 }.into();
    match e {
        LlmError::Timeout { seconds } => assert_eq!(seconds, 12),
        other => panic!("expected Timeout, got {other:?}"),
    }
    let e: LlmError = ClaudeCliError::RateLimited {
        retry_after_seconds: 30,
    }
    .into();
    match e {
        LlmError::RateLimited {
            retry_after_seconds,
        } => assert_eq!(retry_after_seconds, 30),
        other => panic!("expected RateLimited, got {other:?}"),
    }
}
