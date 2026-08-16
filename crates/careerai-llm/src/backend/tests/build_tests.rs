//! Model-picking, forced-resolve, and `Backend::probe` flag-handling
//! tests. The auth-probe-mechanism tests live in `auth_tests.rs`.

use careerai_core::config::BackendChoice;

use crate::backend::build::{pick_default_model, strip_provider_prefix};
use crate::backend::Backend;
#[cfg(any(feature = "live-llm-api", feature = "live-llm-cli"))]
use crate::backend::BackendError;
#[cfg(feature = "live-llm-cli")]
use crate::error::LlmError;

#[allow(unused_imports)] // some helpers are only used under feature gates.
use super::{cache, cfg, ENV_LOCK};

#[test]
fn pick_default_model_prefers_tailor() {
    let mut c = cfg();
    c.tailor_model = "anthropic/claude-sonnet-4-6".into();
    c.parse_resume_model = "anthropic/claude-haiku".into();
    assert_eq!(pick_default_model(&c, crate::rig::Provider::Anthropic), "claude-sonnet-4-6");
}

#[test]
fn pick_default_model_falls_back_to_empty() {
    let c = cfg();
    assert_eq!(pick_default_model(&c, crate::rig::Provider::Anthropic), "");
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
    let prev = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let res = Backend::resolve(BackendChoice::Api, &cfg(), cache()).await;
    // The keyring may have a real entry on a dev box; tolerate a
    // successful build there. We're testing that the resolver doesn't
    // panic on the no-env path.
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
    // `locate_claude_binary` validates the override; a missing path
    // surfaces as the wrapped `BinaryUnusable` (mapped through
    // `BackendError::Llm`). Either is acceptable; a `CliMissing` from
    // the empty-PATH path is also fine on a host without a real `claude`.
    match res {
        Err(BackendError::CliMissing) => {}
        Err(BackendError::Llm(LlmError::Upstream(s))) => {
            assert!(s.to_lowercase().contains("not usable"), "got: {s}");
        }
        other => panic!("expected CliMissing or Llm(Upstream), got {other:?}"),
    }
}

/// `Backend::probe` must honor `CAREERAI_SKIP_CLI_PROBE=1` for parity
/// with `resolve_auto` and `build_cli`.
#[cfg(feature = "live-llm-cli")]
#[tokio::test]
async fn probe_honors_skip_cli_probe_env() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
/// without lying in `chosen`.
#[cfg(feature = "live-llm-cli")]
#[tokio::test]
async fn probe_records_forced_choice_separately_from_chosen() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    // `chosen` must not lie: with no API key reachable, the API backend
    // would not resolve, so `chosen` stays on the auto-detected
    // ClaudeCli (skip env makes the stub binary look healthy). The
    // keyring may have a real key on a dev box; in that case `chosen`
    // is allowed to land on `Api`.
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
