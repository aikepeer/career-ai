//! Subprocess transport and prompt-file handling for CLI-backed LLMs.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{sleep, timeout};
use tracing::trace;

use crate::error::{LlmError, Result};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

use super::driver::{log_no_prompt_cache_once, ClaudeCliLlm};
use super::error::ClaudeCliError;

impl ClaudeCliLlm {
    /// Spawn the subprocess once and capture its parsed JSON result.
    /// Caching is the caller's job (the outer `tailor_for_listing`
    /// `Cache` wrap is the canonical layer; this driver is a thin
    /// transport).
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn send_once(
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

        let file_name = self
            .binary
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        // `agy` (a Claude-Code-compatible Go CLI) accepts `-p/--print`
        // and `--output-format json` but has no `--append-system-prompt-file`
        // flag and reports a different envelope — both handled below.
        let is_agy = file_name.contains("agy");
        // `goose` is an agentic AI CLI driven in headless single-turn
        // mode (`goose run --no-tui --no-session --quiet --output-format
        // json --max-turns 1`). It has no `--append-system-prompt-file`
        // flag and emits a different JSON envelope (`messages` array +
        // `metadata`), both handled below.
        let is_goose = file_name.contains("goose");
        let is_claude = file_name.contains("claude");

        // SECURITY: the system prompt + profile block can carry PII
        // (rendered profile YAML). Passing them via argv would expose
        // that content to any local process via `/proc/<pid>/cmdline`
        // (Linux) or `ps -ef` output. For claude-compatible CLIs, write
        // them to a 0o600 file in a private dir we own, and use
        // `--append-system-prompt-file` so argv carries only flags +
        // model id. `agy` has no such flag AND takes the prompt as an
        // argv argument (stdin prompts make it print flag help instead
        // of responding), so its system text + profile must ride in the
        // prompt argument — an accepted agy-specific trade-off the
        // operator opts into by configuring the backend. `goose` reads
        // the prompt from stdin, so folding system + profile into the
        // stdin prompt keeps PII out of argv entirely.
        let combined = if req.system.is_empty() && req.profile_block.is_empty() {
            String::new()
        } else if req.profile_block.is_empty() {
            req.system.clone()
        } else if req.system.is_empty() {
            req.profile_block.clone()
        } else {
            format!("{}\n\n{}", req.system, req.profile_block)
        };
        // Goose receives trusted system instructions through its dedicated
        // system option. Profile data and the untrusted listing request are
        // kept as explicit data segments in stdin rather than concatenated
        // with the system instruction.
        let folded = is_agy || is_goose;
        let folded_prompt = if is_goose {
            [
                (!req.profile_block.is_empty())
                    .then(|| format!("<PROFILE_BLOCK>\n{}\n</PROFILE_BLOCK>", req.profile_block)),
                (!req.user.is_empty())
                    .then(|| format!("<USER_REQUEST>\n{}\n</USER_REQUEST>", req.user)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n")
        } else if is_agy {
            if combined.is_empty() {
                req.user.clone()
            } else {
                format!("{combined}\n\n{}", req.user)
            }
        } else {
            String::new()
        };

        let mut cmd = Command::new(&self.binary);

        if is_claude {
            cmd.arg("--print")
                .arg("--output-format")
                .arg("json")
                .arg("--model")
                .arg(model);
        } else if is_agy {
            // agy consumes the argument immediately after `-p` as the
            // prompt; flags must come AFTER it (verified against the
            // real binary — flags before the prompt make agy print its
            // `--output-format` help instead of responding).
            cmd.arg("-p")
                .arg(&folded_prompt)
                .arg("--output-format")
                .arg("json");
            // Empty model -> let agy fall back to its own default.
            // A bogus configured model would otherwise be rejected
            // with agy's "invalid model selection" error.
            if !model.is_empty() {
                cmd.arg("--model").arg(model);
            }
        } else if is_goose {
            // goose is an agentic CLI; drive it in headless single-turn
            // mode so it behaves like a one-shot completion:
            //   `--no-session` keeps it non-interactive (no session file),
            //   `--no-profile` disables all MCP extensions/tools so goose
            //     can't loop on tool calls (without it, goose wastes turns
            //     on denied tool attempts or hangs indefinitely),
            //   `--quiet` suppresses the banner so stdout is pure JSON,
            //   `--output-format json` produces a parseable envelope,
            //   `--max-turns 1` enforces one completion rather than an
            //     agent continuation,
            //   `-i -` reads the profile + user data from stdin.
            cmd.arg("run")
                .arg("--no-session")
                .arg("--no-profile")
                .arg("--quiet")
                .arg("--output-format")
                .arg("json")
                .arg("--max-turns")
                .arg("1");
            if !req.system.is_empty() {
                cmd.arg("--system").arg(&req.system);
            }
            if !self.provider.trim().is_empty() {
                cmd.arg("--provider").arg(self.provider.trim());
            }
            if !model.is_empty() {
                cmd.arg("--model").arg(model);
            }
            cmd.arg("-i").arg("-");
        } else if file_name.contains("aider") {
            cmd.arg("--message");
        } else {
            cmd.arg("--print");
        }

        // The temp file is held alive until after `wait_with_output`
        // returns; `_prompt_guard` keeps the `NamedTempFile` in scope so
        // its destructor does not unlink before claude reads it.
        let _prompt_guard: Option<tempfile::NamedTempFile> = if folded || combined.is_empty() {
            None
        } else {
            let tmp = write_private_prompt_file(&combined).map_err(|e| {
                ClaudeCliError::Transport(format!("write system-prompt tempfile: {e}"))
            })?;
            cmd.arg("--append-system-prompt-file").arg(tmp.path());
            Some(tmp)
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
        // write. agy receives its prompt via argv and never reads
        // stdin; still drop the pipe so it sees EOF. goose reads the
        // folded (system + profile + user) prompt from stdin.
        if let Some(mut stdin) = child.stdin.take() {
            if !is_agy {
                let user = if is_goose {
                    folded_prompt.clone()
                } else {
                    req.user.clone()
                };
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

        super::output::parse_cli_output(&stdout, &stderr, output.status.success())
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

        let mut retries = 0_u32;
        loop {
            match self.send_once(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) if err.is_retryable() && retries < self.max_retries => {
                    let exponent = retries.min(4);
                    let delay_ms = 500_u64 * (1_u64 << exponent);
                    retries += 1;
                    sleep(Duration::from_millis(delay_ms)).await;
                }
                Err(err) => return Err(LlmError::from(err)),
            }
        }
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
