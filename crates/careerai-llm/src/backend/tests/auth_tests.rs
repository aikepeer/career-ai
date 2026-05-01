//! Tests for the two-tier `claude auth status` / ping fallback probe.
//! All tests require Unix to write executable shell stubs.

#![cfg(all(unix, feature = "live-llm-cli"))]

use careerai_core::config::BackendChoice;

use crate::backend::Backend;

use super::{cfg, write_stub, ENV_LOCK};

/// `claude auth status` returning `{"loggedIn": true, ...}` is the
/// happy path: probe reports `auth_ok=true` without invoking the
/// model. The stub fails loudly if the inference path is exercised
/// — that would be a regression to the costly $0.08 probe.
#[tokio::test]
async fn probe_uses_auth_status_when_logged_in() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    // The stub handles three call shapes:
    //   `--version`          → harmless version string
    //   `auth status`        → loggedIn=true JSON (zero-cost path)
    //   anything else        → exit 99 (would be the inference path)
    write_stub(
        &bin,
        r#"#!/bin/sh
case "$1" in
  --version) echo "claude 1.0.0"; exit 0 ;;
  auth)
    if [ "$2" = "status" ]; then
      echo '{"loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "max"}'
      exit 0
    fi
    ;;
esac
exit 99
"#,
    );
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert!(
        probe.claude_auth_ok,
        "auth status with loggedIn=true should yield auth_ok"
    );
    assert_eq!(probe.chosen, BackendChoice::ClaudeCli);
    // The auth-status path records elapsed time so the report still
    // shows latency.
    assert!(
        probe.claude_ping_ms.is_some(),
        "auth-status path must record claude_ping_ms"
    );
}

/// `claude auth status` returning `{"loggedIn": false, ...}` must
/// short-circuit: probe reports `auth_ok=false` without falling
/// through to the ping path. The stub exits 99 on any inference
/// invocation to make a regression visible.
#[tokio::test]
async fn probe_reports_not_authenticated_when_logged_in_false() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    write_stub(
        &bin,
        r#"#!/bin/sh
case "$1" in
  --version) echo "claude 1.0.0"; exit 0 ;;
  auth)
    if [ "$2" = "status" ]; then
      echo '{"loggedIn": false, "authMethod": null, "subscriptionType": null}'
      exit 0
    fi
    ;;
esac
exit 99
"#,
    );
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert!(
        !probe.claude_auth_ok,
        "loggedIn=false must yield auth_ok=false"
    );
}

/// When `claude auth status` exits non-zero (older `claude` builds
/// lack the subcommand), the orchestrator falls through to the ping
/// path. The stub here returns a healthy ping JSON, so the fallback
/// should report `auth_ok=true`.
#[tokio::test]
async fn probe_falls_back_to_ping_when_auth_status_fails() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    // `auth status` exits non-zero (simulating an older claude build);
    // `--print ... ping` returns a healthy ping JSON.
    write_stub(
        &bin,
        r#"#!/bin/sh
case "$1" in
  --version) echo "claude 1.0.0"; exit 0 ;;
  auth) exit 1 ;;
  --print)
    cat <<'__PAYLOAD__'
{"type":"result","subtype":"success","is_error":false,"result":"OK","total_cost_usd":0.0,"usage":{"input_tokens":3,"output_tokens":4}}
__PAYLOAD__
    exit 0
    ;;
esac
exit 99
"#,
    );
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert!(
        probe.claude_auth_ok,
        "ping fallback should report auth_ok when ping JSON is healthy"
    );
    assert_eq!(probe.chosen, BackendChoice::ClaudeCli);
}

/// `claude auth status` returns valid JSON but the `loggedIn` field is
/// absent. Defensive parsing should treat the response as unusable and
/// fall through to the ping path. The stub returns a healthy ping so
/// the fallback succeeds, proving the missing-field case did NOT cause
/// a misclassified `auth_ok`.
#[tokio::test]
async fn probe_falls_back_when_auth_status_missing_logged_in_field() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    write_stub(
        &bin,
        r#"#!/bin/sh
case "$1" in
  --version) echo "claude 1.0.0"; exit 0 ;;
  auth)
    if [ "$2" = "status" ]; then
      echo '{"authMethod": "claude.ai", "subscriptionType": "max"}'
      exit 0
    fi
    ;;
  --print)
    cat <<'__PAYLOAD__'
{"type":"result","subtype":"success","is_error":false,"result":"OK","total_cost_usd":0.0,"usage":{"input_tokens":3,"output_tokens":4}}
__PAYLOAD__
    exit 0
    ;;
esac
exit 99
"#,
    );
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert!(
        probe.claude_auth_ok,
        "missing loggedIn must fall through to ping (which succeeded here)"
    );
}

/// `claude auth status` returns a `loggedIn` field that's a string
/// (`"true"`) instead of a JSON boolean. `as_bool()` is strict and
/// rejects this. The orchestrator must fall through to the ping path;
/// here the ping succeeds.
#[tokio::test]
async fn probe_falls_back_when_auth_status_logged_in_is_string() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    write_stub(
        &bin,
        r#"#!/bin/sh
case "$1" in
  --version) echo "claude 1.0.0"; exit 0 ;;
  auth)
    if [ "$2" = "status" ]; then
      echo '{"loggedIn": "true", "authMethod": "claude.ai"}'
      exit 0
    fi
    ;;
  --print)
    cat <<'__PAYLOAD__'
{"type":"result","subtype":"success","is_error":false,"result":"OK","total_cost_usd":0.0,"usage":{"input_tokens":3,"output_tokens":4}}
__PAYLOAD__
    exit 0
    ;;
esac
exit 99
"#,
    );
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert!(
        probe.claude_auth_ok,
        "non-bool loggedIn must fall through to ping"
    );
}

/// Both tiers return unparseable garbage. `auth_ok` must be `false`
/// and `claude_ping_ms` must be `None` (verifies the
/// stale-thread-local fix — no leftover value from a previous
/// successful probe on the same thread).
#[tokio::test]
async fn probe_returns_false_when_both_tiers_unparseable() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    write_stub(
        &bin,
        r#"#!/bin/sh
case "$1" in
  --version) echo "claude 1.0.0"; exit 0 ;;
  auth)
    if [ "$2" = "status" ]; then
      echo "this is not json {{{"
      exit 0
    fi
    ;;
  --print) echo "also not json"; exit 0 ;;
esac
exit 99
"#,
    );
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert!(
        !probe.claude_auth_ok,
        "both tiers unparseable must yield auth_ok=false"
    );
    assert_eq!(
        probe.claude_ping_ms, None,
        "ping_ms must be None when no probe tier succeeded"
    );
}
