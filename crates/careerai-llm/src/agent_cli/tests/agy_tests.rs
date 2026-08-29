#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::await_holding_lock
)]

use std::sync::Arc;

use super::ClaudeCliLlm;
use crate::cache::Cache;
use crate::error::LlmError;
use crate::trait_def::Llm;
use crate::types::LlmRequest;
use crate::ENV_LOCK;

/// `backend: agy` support: agy is a Claude-Code-compatible Go CLI that
/// accepts `-p/--print`, `--output-format json` and `--model`, but has
/// NO `--append-system-prompt-file` flag, reports a different JSON
/// envelope (`{"status":"SUCCESS","response":...}`), and requires the
/// prompt as an ARGV argument (stdin prompts make it print flag help).
/// The driver must speak agy's dialect: no claude-only flag, the
/// folded system+profile+user text in argv, and the agy envelope
/// parsed. The argv placement is an accepted agy-specific trade-off —
/// agy offers no PII-safe prompt channel.
#[tokio::test]
async fn agy_stub_binary_success_path() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("agy");
    let argv_file = dir.path().join("argv.txt");
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > '{}'\n\
         cat <<'__P__'\n\
         {{\"status\":\"SUCCESS\",\"response\":\"AGY-OK\",\"usage\":{{\"input_tokens\":11,\"output_tokens\":4,\"cache_read_tokens\":0}}}}\n\
         __P__\n",
        argv_file.display()
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
    let llm = ClaudeCliLlm::new(&stub_path, "gemini-3.7-flash-medium", cache, 10);
    let req = LlmRequest {
        system: "SYSTEM_TEXT".into(),
        profile_block: "PROFILE_BLOCK".into(),
        user: "USER_TEXT".into(),
        prompt_version: "v".into(),
        model: String::new(),
        temperature: 0.0,
        max_tokens: 1,
        cache_profile: false,
    };
    let resp = llm.complete(&req).await.expect("agy success should parse");
    assert_eq!(resp.text, "AGY-OK");
    assert_eq!(resp.prompt_tokens, 11);
    assert_eq!(resp.completion_tokens, 4);

    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(argv.contains("-p"), "agy argv missing -p: {argv}");
    assert!(
        argv.contains("--output-format"),
        "agy argv missing --output-format: {argv}"
    );
    assert!(
        argv.contains("gemini-3.7-flash-medium"),
        "agy argv missing model: {argv}"
    );
    assert!(
        !argv.contains("--append-system-prompt-file"),
        "agy must not receive the claude-only --append-system-prompt-file: {argv}"
    );
    assert!(
        argv.contains("SYSTEM_TEXT"),
        "system text must reach agy in argv: {argv}"
    );
    assert!(
        argv.contains("PROFILE_BLOCK"),
        "profile block must reach agy in argv: {argv}"
    );
    assert!(
        argv.contains("USER_TEXT"),
        "user prompt must reach agy: {argv}"
    );
    let sys_pos = argv.find("SYSTEM_TEXT").unwrap();
    let user_pos = argv.find("USER_TEXT").unwrap();
    assert!(
        sys_pos < user_pos,
        "system text must precede the user prompt: {argv}"
    );
    // Regression: agy consumes the argument right after `-p` as the
    // prompt, so flags before the prompt make it print `--output-format`
    // help instead of responding (observed against the real binary).
    // The folded prompt must be the arg immediately following `-p`.
    let mut lines = argv.lines();
    assert_eq!(
        lines.next(),
        Some("-p"),
        "agy argv must start with -p: {argv}"
    );
    let prompt_arg = lines.next().expect("prompt must be the arg right after -p");
    assert!(
        prompt_arg.contains("SYSTEM_TEXT"),
        "the arg after -p must be the folded prompt: {argv}"
    );
}

#[tokio::test]
async fn agy_stub_binary_error_envelope() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("agy");
    let script = "#!/bin/sh\ncat <<'__P__'\n{\"status\":\"ERROR\",\"response\":\"\",\"error\":\"invalid model selection: model foo is not recognized\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}\n__P__\n";
    std::fs::write(&stub_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_path, perms).unwrap();
    }

    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let llm = ClaudeCliLlm::new(&stub_path, "foo", cache, 10);
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
        LlmError::Upstream(s) => assert!(s.contains("invalid model selection"), "got: {s}"),
        other => panic!("expected Upstream, got {other:?}"),
    }
}

#[tokio::test]
async fn agy_stub_binary_skips_model_flag_when_unset() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // An empty model must not emit `--model`, letting agy fall back to
    // its own default instead of failing on an unknown model id.
    let dir = tempfile::tempdir().unwrap();
    let stub_path = dir.path().join("agy");
    let argv_file = dir.path().join("argv.txt");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat <<'__P__'\n{{\"status\":\"SUCCESS\",\"response\":\"OK\",\"usage\":{{}}}}\n__P__\n",
        argv_file.display()
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
    let llm = ClaudeCliLlm::new(&stub_path, "", cache, 10);
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
    llm.complete(&req).await.expect("ok");
    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(
        !argv.contains("--model"),
        "empty model must not emit --model: {argv}"
    );
}
