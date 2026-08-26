#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unwrap_in_result)]

use std::sync::Arc;

use super::{fixture_path, ClaudeCliLlm};
use crate::cache::Cache;
use crate::error::LlmError;
use crate::trait_def::Llm;
use crate::types::LlmRequest;
use crate::ENV_LOCK;

/// Stub-binary integration: write a tiny shell script that emits a
/// canned `claude --print` JSON payload, point the driver at it, and
/// verify the parsed `LlmResponse`. Pattern mirrors
/// `tests/mcp_jobs_it.rs::self_respawn`.
#[tokio::test]
async fn stub_binary_success_path() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
    let payload = std::fs::read_to_string(fixture_path("claude_cli_ping.json")).unwrap();
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

    let resp2 = llm.complete(&req).await.expect("ok2");
    assert_eq!(resp2.text, "OK");
    assert!(!resp2.cache_hit);
}

#[tokio::test]
async fn stub_binary_auth_expired() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
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
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
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
        model: "haiku".into(),
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
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("claude");
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
    assert!(
        argv.contains("--append-system-prompt-file"),
        "expected --append-system-prompt-file in argv: {argv}"
    );
}
