#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unwrap_in_result)]

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::cache::Cache;
use crate::error::LlmError;
use crate::trait_def::Llm;
use crate::types::LlmRequest;

/// Serialize tests that mutate `CAREERAI_CLAUDE_BIN` /
/// `CAREERAI_SKIP_CLI_PROBE`. cargo runs tests in parallel by
/// default; without this lock, env-var flips race across threads
/// and produce intermittent failures (the var leaking from one
/// test's set into another's read).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixture_path(name: &str) -> PathBuf {
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

/// Stub-binary integration: write a tiny shell script that emits a
/// canned `claude --print` JSON payload, point the driver at it, and
/// verify the parsed `LlmResponse`. Pattern mirrors
/// `tests/mcp_jobs_it.rs::self_respawn`.
#[tokio::test]
async fn stub_binary_success_path() {
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    let payload = std::fs::read_to_string(fixture_path("claude_cli_ping.json")).unwrap();
    // Use Python-portable shell that is guaranteed to be present.
    let script = format!("#!/bin/sh\ncat <<'__PAYLOAD__'\n{payload}\n__PAYLOAD__\n");
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }

    let cache_dir = dir.path().join("cache");
    let cache = Arc::new(Cache::new(&cache_dir));
    let llm = ClaudeCliLlm::new(&stub_path, "sonnet", cache, 30);

    let req = LlmRequest {
        system: "you are terse".into(),
        profile_block: String::new(),
        user: "ping".into(),
        prompt_version: "tailor.v1".into(),
        model: String::new(),
        temperature: 0.0,
        max_tokens: 64,
        cache_profile: false,
    };
    let resp = llm.complete(&req).await.expect("ok");
    assert_eq!(resp.text, "OK");
    assert_eq!(resp.prompt_tokens, 3);
    assert_eq!(resp.completion_tokens, 4);
    assert!(!resp.cache_hit);

    // Second call also returns ok. The on-disk response cache lives
    // in the outer `tailor_for_listing` wrapper, not in this driver
    // (see `complete()`), so `cache_hit` stays false here.
    let resp2 = llm.complete(&req).await.expect("ok2");
    assert_eq!(resp2.text, "OK");
    assert!(!resp2.cache_hit);
}

#[tokio::test]
async fn stub_binary_auth_expired() {
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    let payload = r#"{"type":"result","is_error":true,"result":"Not logged in · Please run /login","usage":{"input_tokens":0,"output_tokens":0}}"#;
    let script = format!("#!/bin/sh\ncat <<'__PAYLOAD__'\n{payload}\n__PAYLOAD__\nexit 1\n");
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }

    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let llm = ClaudeCliLlm::new(&stub_path, "sonnet", cache, 10);
    let req = LlmRequest {
        system: String::new(),
        profile_block: String::new(),
        user: "x".into(),
        prompt_version: "v".into(),
        model: String::new(),
        temperature: 0.0,
        max_tokens: 1,
        cache_profile: false,
    };
    let err = llm.complete(&req).await.unwrap_err();
    match err {
        LlmError::Upstream(s) => assert!(s.to_lowercase().contains("not authenticated")),
        other => panic!("expected Upstream auth error, got {other:?}"),
    }
}

#[tokio::test]
async fn stub_binary_malformed_json() {
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    let script = "#!/bin/sh\necho 'this is not json'\n";
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }
    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let llm = ClaudeCliLlm::new(&stub_path, "sonnet", cache, 10);
    let req = LlmRequest {
        system: String::new(),
        profile_block: String::new(),
        user: "x".into(),
        prompt_version: "v".into(),
        model: String::new(),
        temperature: 0.0,
        max_tokens: 1,
        cache_profile: false,
    };
    let err = llm.complete(&req).await.unwrap_err();
    match err {
        LlmError::Schema(s) => assert!(s.contains("claude-cli")),
        other => panic!("expected Schema, got {other:?}"),
    }
}

#[tokio::test]
async fn stub_binary_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    // Sleep longer than the driver's timeout.
    let script = "#!/bin/sh\nsleep 5\necho '{}'\n";
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }
    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let llm = ClaudeCliLlm::new(&stub_path, "sonnet", cache, 1);
    let req = LlmRequest {
        system: String::new(),
        profile_block: String::new(),
        user: "x".into(),
        prompt_version: "v".into(),
        model: String::new(),
        temperature: 0.0,
        max_tokens: 1,
        cache_profile: false,
    };
    let err = llm.complete(&req).await.unwrap_err();
    match err {
        LlmError::Timeout { seconds } => assert!(seconds <= 2),
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn stub_binary_passes_model_flag() {
    // Stub script echoes back its argv as JSON: we look for `--model
    // sonnet` to verify alias passthrough.
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    // The script ignores stdin and emits a fixed success payload,
    // but writes its argv to a sentinel file we can inspect.
    let sentinel = dir.path().join("argv.txt");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat <<'__P__'\n{{\"is_error\":false,\"result\":\"OK\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}\n__P__\n",
        sentinel.display()
    );
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }
    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let llm = ClaudeCliLlm::new(&stub_path, "sonnet", cache, 10);
    let req = LlmRequest {
        system: "s".into(),
        profile_block: String::new(),
        user: "x".into(),
        prompt_version: "v".into(),
        model: "haiku".into(), // request override should win
        temperature: 0.0,
        max_tokens: 1,
        cache_profile: false,
    };
    llm.complete(&req).await.expect("ok");
    let argv = std::fs::read_to_string(&sentinel).unwrap();
    assert!(argv.contains("--model"), "argv missing --model: {argv}");
    assert!(argv.contains("haiku"), "argv missing haiku: {argv}");
}

/// SECURITY regression test: profile content (PII) must NOT appear
/// in the subprocess argv. Argv is visible to other local users via
/// `/proc/<pid>/cmdline` or `ps -ef`, so the system prompt + profile
/// block ride on a 0o600 file referenced by
/// `--append-system-prompt-file`. The argv carries only flags + the
/// temp-file path.
#[tokio::test]
async fn stub_binary_does_not_leak_profile_to_argv() {
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    let sentinel = dir.path().join("argv.txt");
    // The stub records its argv to `sentinel` and emits a canned OK
    // payload. The system prompt content is intentionally a
    // distinctive PII-shaped string so we can assert its absence.
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat <<'__P__'\n{{\"is_error\":false,\"result\":\"OK\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}\n__P__\n",
        sentinel.display()
    );
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Mirror `stub_binary_passes_model_flag`: go through
        // `metadata().permissions()` first to nudge the kernel into
        // releasing any lingering write fd before exec. Avoids the
        // ETXTBSY race we hit with the more direct setter.
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }

    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let llm = ClaudeCliLlm::new(&stub_path, "sonnet", cache, 10);
    let pii = "SECRET_PROFILE_EMAIL: capitalbluecity@example.org PHONE: 9999999999";
    let req = LlmRequest {
        system: "you are terse".into(),
        profile_block: pii.into(),
        user: "go".into(),
        prompt_version: "v".into(),
        model: String::new(),
        temperature: 0.0,
        max_tokens: 1,
        cache_profile: false,
    };
    llm.complete(&req).await.expect("ok");
    let argv = std::fs::read_to_string(&sentinel).unwrap();
    assert!(
        !argv.contains("SECRET_PROFILE_EMAIL"),
        "profile content leaked to argv: {argv}"
    );
    assert!(
        !argv.contains("you are terse"),
        "system prompt leaked to argv: {argv}"
    );
    // The flag itself must be present so the prompt actually
    // reaches claude.
    assert!(
        argv.contains("--append-system-prompt-file"),
        "expected --append-system-prompt-file in argv: {argv}"
    );
}
