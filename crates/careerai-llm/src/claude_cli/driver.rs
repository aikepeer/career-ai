//! Subprocess-backed `Llm` impl that shells out to the user's
//! `claude` CLI (`--print --output-format json`).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, trace};

use crate::cache::Cache;
use crate::error::{LlmError, Result};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

use super::binary_locator::locate_claude_binary;
use super::error::{classify_error_payload, classify_failure_stderr, ClaudeCliError};
use super::response::{clamp_u64_u32, ClaudeCliResult};

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
    /// resolved, or [`ClaudeCliError::BinaryUnusable`] when the resolved
    /// path is not a regular executable file.
    pub fn discover(
        model: impl Into<String>,
        cache: Arc<Cache>,
        timeout_seconds: u64,
    ) -> std::result::Result<Self, ClaudeCliError> {
        let binary = locate_claude_binary()?;
        Ok(Self::new(binary, model, cache, timeout_seconds))
    }

    /// Spawn the subprocess once and capture its parsed JSON result.
    /// Caching is the caller's job (the outer `tailor_for_listing`
    /// `Cache` wrap is the canonical layer; this driver is a thin
    /// transport).
    #[allow(clippy::too_many_lines)]
    async fn send_once(
        &self,
        req: &LlmRequest,
    ) -> std::result::Result<LlmResponse, ClaudeCliError> {
        let model_raw = if req.model.is_empty() {
            self.model.as_str()
        } else {
            req.model.as_str()
        };
        // Strip a leading `anthropic/` namespace; the `claude` CLI
        // rejects rig-style multi-provider names. `pick_default_model`
        // strips for the backend default; the per-request path lives
        // here.
        let model = model_raw.split_once('/').map_or(model_raw, |(_, r)| r);

        let bin_str = self.binary.to_string_lossy().to_lowercase();
        let mut cmd = Command::new(&self.binary);

        if bin_str.contains("claude") {
            cmd.arg("--print")
                .arg("--output-format")
                .arg("json")
                .arg("--model")
                .arg(model);
        } else if bin_str.contains("agy") {
            cmd.arg("-p").arg("--print");
        } else if bin_str.contains("goose") {
            cmd.arg("run");
        } else if bin_str.contains("aider") {
            cmd.arg("--message");
        } else {
            cmd.arg("--print");
        }

        // SECURITY: the system prompt + profile block can carry PII
        // (rendered profile YAML). Passing them via argv would expose
        // that content to any local process via `/proc/<pid>/cmdline`
        // (Linux) or `ps -ef` output. Write them to a 0o600 file in a
        // private dir we own, and use `--append-system-prompt-file` so
        // argv carries only flags + model id.
        //
        // The temp file is held alive until after `wait_with_output`
        // returns; `_prompt_guard` keeps the `NamedTempFile` in scope so
        // its destructor does not unlink before claude reads it.
        let _prompt_guard: Option<tempfile::NamedTempFile> =
            if !req.system.is_empty() || !req.profile_block.is_empty() {
                let combined = if req.profile_block.is_empty() {
                    req.system.clone()
                } else if req.system.is_empty() {
                    req.profile_block.clone()
                } else {
                    format!("{}\n\n{}", req.system, req.profile_block)
                };
                let tmp = write_private_prompt_file(&combined).map_err(|e| {
                    ClaudeCliError::Transport(format!("write system-prompt tempfile: {e}"))
                })?;
                cmd.arg("--append-system-prompt-file").arg(tmp.path());
                Some(tmp)
            } else {
                None
            };

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

        // The on-disk response cache lives in the outer
        // `tailor_for_listing` wrapper (see `careerai-tailor::lib`),
        // keyed by `compose_key(prompt_version, profile_hash, jd_hash,
        // model)`. That layer is provider-agnostic and shared across
        // CLI + API backends. This driver intentionally does NOT add a
        // second on-disk cache layer with its own key shape — having
        // two layers writing to the same dir caused duplicate I/O and
        // surprised auditors.
        //
        // The `cache` field is retained on the struct for API
        // compatibility (`Backend::resolve` still hands one in), but
        // unused at the call site below.
        let _ = &self.cache;

        let resp = self.send_once(req).await.map_err(LlmError::from)?;
        Ok(resp)
    }
}

/// Write `combined` to a 0o600 temp file in a private parent dir. The
/// `--append-system-prompt-file` flag on `claude` reads the contents at
/// startup; the caller keeps the returned `NamedTempFile` alive across
/// the spawn so the destructor doesn't unlink it before claude reads.
///
/// SECURITY: prompts can carry rendered profile YAML (PII). We anchor
/// the parent dir under `target/.careerai-prompts/` (project-local,
/// `.gitignore`'d) when running inside a cargo workspace, falling back
/// to `<system-tempdir>/careerai-prompts/` otherwise. Both the dir and
/// the file get owner-only Unix modes; no predictable filename in
/// world-readable `/tmp`.
fn write_private_prompt_file(combined: &str) -> std::io::Result<tempfile::NamedTempFile> {
    use std::io::Write;

    let parent = if std::path::Path::new("target").is_dir() {
        std::path::PathBuf::from("target").join(".careerai-prompts")
    } else {
        std::env::temp_dir().join("careerai-prompts")
    };
    std::fs::create_dir_all(&parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort tighten parent dir to owner-only. Ignore errors:
        // a pre-existing dir we don't own would surface later via the
        // tempfile open, with a clearer error.
        let _ = std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700));
    }

    let mut tmp = tempfile::Builder::new()
        .prefix("prompt-")
        .suffix(".txt")
        .tempfile_in(&parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let f = tmp.as_file();
        let mut perms = f.metadata()?.permissions();
        perms.set_mode(0o600);
        f.set_permissions(perms)?;
    }
    tmp.write_all(combined.as_bytes())?;
    tmp.as_file_mut().sync_all()?;
    Ok(tmp)
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
