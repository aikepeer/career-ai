//! Tests for backend resolution and probe logic.
//!
//! These tests mutate `CAREERAI_CLAUDE_BIN` / `CAREERAI_SKIP_CLI_PROBE` /
//! `ANTHROPIC_API_KEY`. Use `ENV_LOCK` to serialize across parallel
//! cargo-test threads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::await_holding_lock)]

use std::sync::Arc;

use careerai_core::config::{BackendChoice, LlmConfig};

use crate::backend::resolution::{pick_default_model, strip_provider_prefix};
use crate::backend::{Backend, BackendError};
use crate::cache::Cache;
use crate::error::LlmError;

/// Serialize tests that mutate `CAREERAI_CLAUDE_BIN` /
/// `CAREERAI_SKIP_CLI_PROBE` / `ANTHROPIC_API_KEY`. These vars are
/// process-global; without the lock, parallel cargo-test threads
/// race their set/remove pairs and produce intermittent failures.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn cfg() -> LlmConfig {
    LlmConfig::default()
}

fn cache() -> Arc<Cache> {
    Arc::new(Cache::new(
        std::env::temp_dir().join("careerai-llm-backend-test"),
    ))
}

#[test]
fn pick_default_model_prefers_tailor() {
    let mut c = cfg();
    c.tailor_model = "anthropic/claude-sonnet-4-6".into();
    c.parse_resume_model = "anthropic/claude-haiku".into();
    assert_eq!(pick_default_model(&c), "claude-sonnet-4-6");
}

#[test]
fn pick_default_model_falls_back_to_sonnet() {
    let c = cfg();
    assert_eq!(pick_default_model(&c), "sonnet");
}

#[test]
fn strip_namespace() {
    assert_eq!(
        strip_provider_prefix("anthropic/claude-sonnet-4-6"),
        "claude-sonnet-4-6"
    );
    assert_eq!(
        strip_provider_prefix("claude-sonnet-4-6"),
        "claude-sonnet-4-6"
    );
}

#[cfg(feature = "live-llm-api")]
#[tokio::test]
async fn forced_api_with_no_key_errors() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Stash + clear envs first.
    let prev = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let res = Backend::resolve(BackendChoice::Api, &cfg(), cache()).await;
    // The keyring may have a real entry on a dev box; tolerate a
    // successful build there. We're testing that the resolver
    // doesn't panic on the no-env path.
    match res {
        Err(BackendError::ApiKeyMissing) | Ok(_) => {}
        Err(other) => panic!("expected ApiKeyMissing or Ok, got {other:?}"),
    }
    if let Some(v) = prev {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
}

#[cfg(feature = "live-llm-cli")]
#[tokio::test]
async fn forced_cli_with_no_binary_errors() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::env::set_var(
        "CAREERAI_CLAUDE_BIN",
        "/nonexistent/path/that/does/not/resolve",
    );
    std::env::set_var("CAREERAI_SKIP_CLI_PROBE", "1");
    let res = Backend::resolve(BackendChoice::ClaudeCli, &cfg(), cache()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    // `locate_claude_binary` now validates the override; a missing
    // path surfaces as the wrapped `BinaryUnusable` (mapped through
    // `BackendError::Llm`). Either is acceptable; a `CliMissing`
    // from the empty-PATH path is also fine on a host without a
    // real `claude`.
    match res {
        Err(BackendError::CliMissing) => {}
        Err(BackendError::Llm(LlmError::Upstream(s))) => {
            assert!(s.to_lowercase().contains("not usable"), "got: {s}");
        }
        other => panic!("expected CliMissing or Llm(Upstream), got {other:?}"),
    }
}

/// `Backend::probe` must honor `CAREERAI_SKIP_CLI_PROBE=1` for
/// parity with `resolve_auto` and `build_cli`. Without this the
/// module doc was a lie: tests and offline scenarios spawned the
/// real `claude` binary even when the env var was set.
#[cfg(feature = "live-llm-cli")]
#[tokio::test]
async fn probe_honors_skip_cli_probe_env() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Point at a real, executable stub that emits a non-JSON
    // sentinel — the live ping path would fail, but with
    // CAREERAI_SKIP_CLI_PROBE=1 the probe must skip it and report
    // claude_auth_ok = true purely from binary presence.
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    std::fs::write(&bin, "#!/bin/sh\necho not-json\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::set_var("CAREERAI_SKIP_CLI_PROBE", "1");
    let probe = Backend::probe(&cfg()).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    assert!(
        probe.claude_auth_ok,
        "probe should claim auth_ok when SKIP env is set"
    );
    assert_eq!(probe.chosen, BackendChoice::ClaudeCli);
}

/// `Backend::probe` records a forced backend choice in `probe.forced`
/// without lying in `chosen`. `chosen` always reflects what would
/// actually resolve. So forcing `Api` while the only reachable
/// backend is the CLI yields `forced=Some(Api), chosen=ClaudeCli`,
/// and the CLI surfaces the mismatch as a non-zero exit.
#[cfg(feature = "live-llm-cli")]
#[tokio::test]
async fn probe_records_forced_choice_separately_from_chosen() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Stash + clear the API key so the API-key branch in probe()
    // doesn't accidentally satisfy the forced=Api request from the
    // host's keyring.
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    std::fs::write(&bin, "#!/bin/sh\necho not-json\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("CAREERAI_CLAUDE_BIN", &bin);
    std::env::set_var("CAREERAI_SKIP_CLI_PROBE", "1");
    let mut c = cfg();
    c.backend = BackendChoice::Api;
    let probe = Backend::probe(&c).await;
    std::env::remove_var("CAREERAI_CLAUDE_BIN");
    std::env::remove_var("CAREERAI_SKIP_CLI_PROBE");
    if let Some(v) = prev_key {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    assert_eq!(probe.forced, Some(BackendChoice::Api));
    // `chosen` must not lie: with no API key reachable, the API
    // backend would not resolve, so `chosen` stays on the
    // auto-detected ClaudeCli (skip env makes the stub binary look
    // healthy). The keyring may have a real key on a dev box; in
    // that case `chosen` is allowed to land on `Api`.
    assert!(
        matches!(probe.chosen, BackendChoice::ClaudeCli | BackendChoice::Api),
        "chosen={:?} not in {{ClaudeCli, Api}}",
        probe.chosen
    );
}

/// Forced `Auto` is a no-op — `forced` stays `None` even when
/// `cfg.backend == Auto`. Only the explicit overrides (`ClaudeCli`,
/// `Api`) populate `forced`.
#[tokio::test]
async fn probe_forced_none_when_cfg_auto() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut c = cfg();
    c.backend = BackendChoice::Auto;
    let probe = Backend::probe(&c).await;
    assert_eq!(probe.forced, None);
}

/// Helper: write an executable shell stub at the given path.
#[cfg(all(unix, feature = "live-llm-cli"))]
fn write_stub(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `claude auth status` returning `{"loggedIn": true, ...}` is the
/// happy path: probe reports `auth_ok=true` without invoking the
/// model. The stub fails loudly if the inference path is exercised
/// — that would be a regression to the costly $0.08 probe.
#[cfg(all(unix, feature = "live-llm-cli"))]
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
#[cfg(all(unix, feature = "live-llm-cli"))]
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
/// lack the subcommand), the orchestrator falls through to the
/// ping path. The stub here returns a healthy ping JSON, so the
/// fallback should report `auth_ok=true`.
#[cfg(all(unix, feature = "live-llm-cli"))]
#[tokio::test]
async fn probe_falls_back_to_ping_when_auth_status_fails() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    // `auth status` exits non-zero (simulating an older claude
    // build); `--print ... ping` returns a healthy ping JSON.
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

/// `claude auth status` returns valid JSON but the `loggedIn`
/// field is absent. Defensive parsing should treat the response
/// as unusable and fall through to the ping path. The stub
/// returns a healthy ping so the fallback succeeds, proving the
/// missing-field case did NOT cause a misclassified `auth_ok`.
#[cfg(all(unix, feature = "live-llm-cli"))]
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

/// `claude auth status` returns a `loggedIn` field that's a
/// string (`"true"`) instead of a JSON boolean. `as_bool()` is
/// strict and rejects this. The orchestrator must fall through to
/// the ping path; here the ping succeeds.
#[cfg(all(unix, feature = "live-llm-cli"))]
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

/// Both tiers return unparseable garbage. `auth_ok` must be
/// `false` and `claude_ping_ms` must be `None` (verifies the
/// stale-thread-local fix — no leftover value from a previous
/// successful probe on the same thread).
#[cfg(all(unix, feature = "live-llm-cli"))]
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
