//! `claude` CLI subprocess backend for the [`Llm`] trait.
//!
//! Runs the user's locally-installed `claude` binary (Anthropic's Claude
//! Code CLI) in `--print --output-format json` mode and parses its result.
//! When the user is logged in via `claude login` (Max/Pro subscription or
//! API key), every call here bills against that session — no
//! `ANTHROPIC_API_KEY` is required in the environment.
//!
//! # `claude --print --output-format json` shape (verified against
//! `claude` v2.1.119, captured 2026-04-27 — see
//! `tests/fixtures/claude_cli_ping.json`)
//!
//! Exit code is `0` on success, `1` on any error (auth missing, invalid
//! API key, transport, etc.). The CLI emits a *single* JSON object on
//! stdout (NOT NDJSON) regardless of success/failure. Relevant fields:
//!
//! ```json
//! {
//!   "type": "result",
//!   "subtype": "success",
//!   "is_error": false,                 // true on auth/transport/rate failures
//!   "api_error_status": null,          // 401 for "Invalid API key", etc.
//!   "result": "OK",                    // <-- the response text
//!   "stop_reason": "end_turn",
//!   "usage": {
//!     "input_tokens": 3,
//!     "output_tokens": 4,
//!     "cache_creation_input_tokens": 37858,
//!     "cache_read_input_tokens": 0
//!   },
//!   "modelUsage": {
//!     "claude-sonnet-4-6": { ... }     // resolved-model alias keys
//!   }
//! }
//! ```
//!
//! Failure variants seen in the wild:
//!
//! - Not logged in (no creds at all): exit 1, JSON has
//!   `"is_error": true, "result": "Not logged in · Please run /login"`,
//!   `api_error_status: null`, all `usage` fields zero.
//! - Invalid API key (when `ANTHROPIC_API_KEY` is set but bad, with
//!   `--bare`): exit 1, `is_error: true`, `api_error_status: 401`,
//!   `result: "Invalid API key · Fix external API key"`.
//!
//! Model aliases (`sonnet`, `haiku`, `opus`) and full ids
//! (`claude-sonnet-4-6`) are both accepted by `--model`.
//!
//! # Caching
//!
//! Anthropic's prompt-cache `cache_control` blocks are an API-only feature
//! and are not exposed by the CLI surface; `LlmRequest::cache_profile=true`
//! is a no-op here (logged once via `tracing::debug!`). On-disk
//! [`Cache`] still works because it keys on the same
//! `(prompt_version, profile_hash, jd_hash, model)` tuple as the API path.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, trace};

use sha2::{Digest, Sha256};

use crate::cache::{Cache, CacheKey};
use crate::error::{LlmError, Result};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

/// Errors specific to the `claude` CLI subprocess driver.
///
/// Mapped into [`LlmError`] at the trait boundary via `From`.
#[derive(Debug, thiserror::Error)]
pub enum ClaudeCliError {
    #[error("`claude` binary not found on PATH (install Claude Code or set CAREERAI_CLAUDE_BIN)")]
    NotInstalled,

    #[error("claude CLI session is not authenticated; run `claude login` (or `/login` in claude)")]
    AuthExpired,

    #[error("claude CLI rate-limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("claude CLI transport error: {0}")]
    Transport(String),

    #[error("claude CLI returned malformed JSON: {0}")]
    ParseJson(String),

    #[error("claude CLI timed out after {seconds}s")]
    Timeout { seconds: u64 },
}

impl From<ClaudeCliError> for LlmError {
    fn from(value: ClaudeCliError) -> Self {
        match value {
            ClaudeCliError::NotInstalled | ClaudeCliError::AuthExpired => {
                LlmError::Upstream(value.to_string())
            }
            ClaudeCliError::RateLimited {
                retry_after_seconds,
            } => LlmError::RateLimited {
                retry_after_seconds,
            },
            ClaudeCliError::Transport(msg) => LlmError::Upstream(format!("claude-cli: {msg}")),
            ClaudeCliError::ParseJson(msg) => LlmError::Schema(format!("claude-cli: {msg}")),
            ClaudeCliError::Timeout { seconds } => LlmError::Timeout { seconds },
        }
    }
}

/// Subprocess-backed `Llm` impl that shells out to the user's
/// `claude` CLI (`--print --output-format json`).
pub struct ClaudeCliLlm {
    binary: PathBuf,
    model: String,
    cache: Arc<Cache>,
    timeout: Duration,
}

impl std::fmt::Debug for ClaudeCliLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCliLlm")
            .field("binary", &self.binary)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl ClaudeCliLlm {
    /// Construct a driver pointing at `binary` (typically
    /// `which("claude")` resolved). `model` may be either an alias
    /// (`sonnet`, `haiku`, `opus`) or a full model id
    /// (`claude-sonnet-4-6`); the CLI accepts both via `--model`.
    pub fn new(
        binary: impl Into<PathBuf>,
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> Self {
        Self {
            binary: binary.into(),
            model: model.into(),
            cache,
            timeout: Duration::from_secs(timeout_seconds.max(1)),
        }
    }

    /// Convenience constructor: probe `PATH` for `claude` (or honor the
    /// `CAREERAI_CLAUDE_BIN` env override used by tests).
    ///
    /// # Errors
    /// Returns [`ClaudeCliError::NotInstalled`] when the binary cannot be
    /// resolved.
    pub fn discover(
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> std::result::Result<Self, ClaudeCliError> {
        let binary = locate_claude_binary()?;
        Ok(Self::new(binary, model, cache, timeout_seconds))
    }

    /// Spawn the subprocess once and capture its parsed JSON result.
    /// Caching is the caller's job (see [`Self::complete`]).
    async fn send_once(
        &self,
        req: &LlmRequest,
    ) -> std::result::Result<LlmResponse, ClaudeCliError> {
        // Build args. We pipe the user prompt through stdin to avoid
        // argv length limits and shell-escaping pitfalls; the system
        // prompt + profile_block are short enough to ride on flags.
        let model = if req.model.is_empty() {
            self.model.as_str()
        } else {
            req.model.as_str()
        };

        let mut cmd = Command::new(&self.binary);
        cmd.arg("--print")
            .arg("--output-format")
            .arg("json")
            .arg("--model")
            .arg(model);

        // System prompt + profile block both go in via
        // `--append-system-prompt`. The CLI accepts repeated flags
        // (verified empirically); we send them as one concatenated
        // string to stay version-tolerant.
        if !req.system.is_empty() || !req.profile_block.is_empty() {
            let combined = if req.profile_block.is_empty() {
                req.system.clone()
            } else if req.system.is_empty() {
                req.profile_block.clone()
            } else {
                format!("{}\n\n{}", req.system, req.profile_block)
            };
            cmd.arg("--append-system-prompt").arg(combined);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ClaudeCliError::NotInstalled,
            _ => ClaudeCliError::Transport(format!("spawn claude: {e}")),
        })?;

        // Pipe the user prompt on stdin. A broken pipe here is NOT
        // fatal: the child may have exited early (e.g. an auth-fail
        // path that prints its JSON and exits before reading stdin).
        // Surface the child's exit + stdout instead of failing the
        // write.
        if let Some(mut stdin) = child.stdin.take() {
            let user = req.user.clone();
            match stdin.write_all(user.as_bytes()).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                    trace!(
                        target = "careerai_llm::claude_cli",
                        "claude closed stdin before we wrote (likely early exit); continuing"
                    );
                }
                Err(e) => {
                    return Err(ClaudeCliError::Transport(format!("write stdin: {e}")));
                }
            }
            // Drop stdin so the CLI sees EOF.
            drop(stdin);
        }

        // Wait with a timeout. `kill_on_drop` ensures the child is
        // SIGKILL'd if we abandon the future.
        let secs = self.timeout.as_secs();
        let output = match timeout(self.timeout, child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                return Err(ClaudeCliError::Transport(format!("wait child: {e}")));
            }
            Err(_) => {
                return Err(ClaudeCliError::Timeout { seconds: secs });
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        trace!(
            target = "careerai_llm::claude_cli",
            exit_code = output.status.code().unwrap_or(-1),
            stdout_len = stdout.len(),
            stderr_len = stderr.len(),
            "claude --print returned"
        );

        // The CLI emits its result JSON on stdout even on error
        // (`is_error: true`). Parse first; fall back to stderr-based
        // classification only if stdout isn't valid JSON.
        let parsed: ClaudeCliResult = match serde_json::from_str::<ClaudeCliResult>(stdout.trim()) {
            Ok(p) => p,
            Err(e) => {
                if !output.status.success() {
                    // No JSON at all: most likely binary missing or
                    // crashed before printing.
                    return Err(classify_failure_stderr(&stderr));
                }
                return Err(ClaudeCliError::ParseJson(format!(
                    "{e}; stdout={}",
                    stdout.chars().take(256).collect::<String>()
                )));
            }
        };

        if parsed.is_error.unwrap_or(false) {
            return Err(classify_error_payload(&parsed));
        }

        let text = parsed
            .result
            .clone()
            .ok_or_else(|| ClaudeCliError::ParseJson("missing `result` field".into()))?;

        let usage = parsed.usage.unwrap_or_default();

        Ok(LlmResponse {
            text,
            prompt_tokens: clamp_u64_u32(usage.input_tokens),
            completion_tokens: clamp_u64_u32(usage.output_tokens),
            cache_hit: false,
            cached_prompt_tokens: clamp_u64_u32(usage.cache_read_input_tokens),
        })
    }
}

#[async_trait]
impl Llm for ClaudeCliLlm {
    fn name(&self) -> &'static str {
        "claude-cli"
    }

    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        if req.cache_profile {
            log_no_prompt_cache_once();
        }

        let key = cache_key_for(req);
        if let Some(hit) = self.cache.get(&key).await? {
            return Ok(hit);
        }

        let resp = self.send_once(req).await.map_err(LlmError::from)?;
        self.cache.put(&key, &resp).await?;
        Ok(resp)
    }
}

/// Compose a provider-agnostic cache key from the request fields. Uses
/// the same delimiter strategy as [`compose_key`] but operates directly
/// on the (string-typed) `LlmRequest` so this driver doesn't have to
/// reach into `careerai-profile` for a typed `Profile`.
fn cache_key_for(req: &LlmRequest) -> CacheKey {
    let mut h = Sha256::new();
    let model_used = if req.model.is_empty() {
        "default"
    } else {
        req.model.as_str()
    };
    h.update(b"claude-cli\x1F");
    h.update(req.prompt_version.as_bytes());
    h.update([0x1F]);
    h.update(req.system.as_bytes());
    h.update([0x1F]);
    h.update(req.profile_block.as_bytes());
    h.update([0x1F]);
    h.update(req.user.as_bytes());
    h.update([0x1F]);
    h.update(model_used.as_bytes());
    CacheKey::new(hex::encode(h.finalize()))
}

/// One-shot debug log noting that prompt caching isn't available on the
/// CLI backend. Repeated `cache_profile=true` requests across the same
/// process don't spam the log.
fn log_no_prompt_cache_once() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        debug!(
            target = "careerai_llm::claude_cli",
            "anthropic prompt cache unavailable on claude-cli backend (cache_profile flag is silently ignored)"
        );
    });
}

/// Resolve the `claude` binary using the `CAREERAI_CLAUDE_BIN` override
/// (used by tests) before falling back to a `PATH` lookup via the
/// `which` crate. Returns [`ClaudeCliError::NotInstalled`] when neither
/// resolves.
pub(crate) fn locate_claude_binary() -> std::result::Result<PathBuf, ClaudeCliError> {
    if let Ok(p) = std::env::var("CAREERAI_CLAUDE_BIN") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    which::which("claude").map_err(|_| ClaudeCliError::NotInstalled)
}

/// Best-effort classifier for stderr text when stdout had no JSON.
fn classify_failure_stderr(stderr: &str) -> ClaudeCliError {
    let lc = stderr.to_lowercase();
    if lc.contains("not logged in") || lc.contains("/login") {
        ClaudeCliError::AuthExpired
    } else if lc.contains("rate") && lc.contains("limit") {
        ClaudeCliError::RateLimited {
            retry_after_seconds: 60,
        }
    } else if stderr.is_empty() {
        ClaudeCliError::Transport("empty stdout/stderr from claude".into())
    } else {
        // Trim noisy prefix; cap length.
        let snippet: String = stderr.chars().take(256).collect();
        ClaudeCliError::Transport(snippet)
    }
}

/// Classify a parsed-but-errorful JSON payload.
fn classify_error_payload(parsed: &ClaudeCliResult) -> ClaudeCliError {
    let msg = parsed.result.clone().unwrap_or_default();
    let lc = msg.to_lowercase();

    if let Some(status) = parsed.api_error_status {
        match status {
            401 | 403 => return ClaudeCliError::AuthExpired,
            429 => {
                return ClaudeCliError::RateLimited {
                    retry_after_seconds: 60,
                };
            }
            _ => {}
        }
    }

    if lc.contains("not logged in") || lc.contains("/login") || lc.contains("invalid api key") {
        return ClaudeCliError::AuthExpired;
    }
    if lc.contains("rate") && lc.contains("limit") {
        return ClaudeCliError::RateLimited {
            retry_after_seconds: 60,
        };
    }

    let snippet: String = msg.chars().take(256).collect();
    ClaudeCliError::Transport(if snippet.is_empty() {
        "claude reported error with no message".into()
    } else {
        snippet
    })
}

fn clamp_u64_u32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Mirror of the relevant fields in the `claude --print --output-format json`
/// payload. Defensive: every field is `Option`, unknown fields tolerated.
#[derive(Debug, Clone, Default, Deserialize)]
struct ClaudeCliResult {
    #[serde(default)]
    is_error: Option<bool>,
    #[serde(default)]
    api_error_status: Option<u16>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    usage: Option<ClaudeCliUsage>,
    /// Catch-all so unknown keys (e.g. `modelUsage`, `terminal_reason`)
    /// don't break parsing.
    #[serde(flatten, default)]
    #[allow(dead_code)]
    extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[allow(clippy::struct_field_names)] // mirrors the upstream JSON shape
struct ClaudeCliUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    #[allow(dead_code)]
    cache_creation_input_tokens: u64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::unwrap_in_result)]
mod tests {
    use super::*;

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

    #[test]
    fn locate_binary_via_env_override() {
        std::env::set_var("CAREERAI_CLAUDE_BIN", "/usr/local/bin/claude-fake");
        let p = locate_claude_binary().unwrap();
        assert_eq!(p, PathBuf::from("/usr/local/bin/claude-fake"));
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

        // Second call should hit cache.
        let resp2 = llm.complete(&req).await.expect("ok2");
        assert_eq!(resp2.text, "OK");
        assert!(resp2.cache_hit);
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
}
