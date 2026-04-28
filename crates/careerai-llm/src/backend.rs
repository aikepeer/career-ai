//! Backend resolver — picks between the `claude` CLI subprocess driver
//! and the rig-core Anthropic API driver based on a [`BackendChoice`]
//! and what's reachable on the host.
//!
//! Resolution order when [`BackendChoice::Auto`]:
//!
//! 1. `which("claude")` succeeds AND a two-tier auth probe passes
//!    (skipped when `CAREERAI_SKIP_CLI_PROBE=1`) -> [`Backend::ClaudeCli`].
//!    The probe tries `claude auth status` first (cheap, <2s, zero
//!    token cost) and falls back to `claude --print "ping"` with a
//!    60s ceiling for older `claude` builds without `auth status`.
//! 2. `ANTHROPIC_API_KEY` is reachable (env or
//!    `keyring::Entry::new("career-ai", "anthropic/api_key")`) AND the
//!    `live-llm-api` feature is enabled -> [`Backend::Api`].
//! 3. Else [`BackendError::NoneAvailable`] with hints.
//!
//! When forced (`ClaudeCli` or `Api`): no fallback; mismatch is a hard
//! error.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use careerai_core::config::{BackendChoice, LlmConfig};
use tracing::debug;
#[cfg(feature = "live-llm-api")]
use tracing::info;
#[cfg(feature = "live-llm-cli")]
use {std::time::Duration, tokio::process::Command, tokio::time::timeout};

use crate::cache::Cache;
use crate::error::{LlmError, Result as LlmResult};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

#[cfg(feature = "live-llm-cli")]
use crate::claude_cli::{locate_claude_binary, ClaudeCliError, ClaudeCliLlm};

#[cfg(feature = "live-llm-api")]
use crate::rig::{Provider, RigLlm};

/// Resolved backend, ready to issue LLM calls.
pub enum Backend {
    /// `claude` CLI subprocess.
    #[cfg(feature = "live-llm-cli")]
    ClaudeCli(ClaudeCliLlm),
    /// rig-core Anthropic API client.
    #[cfg(feature = "live-llm-api")]
    Api(RigLlm),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "live-llm-cli")]
            Self::ClaudeCli(_) => f.debug_tuple("Backend::ClaudeCli").finish(),
            #[cfg(feature = "live-llm-api")]
            Self::Api(_) => f.debug_tuple("Backend::Api").finish(),
            #[allow(unreachable_patterns)]
            _ => f.write_str("Backend::None"),
        }
    }
}

/// Errors specific to backend resolution.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error(
        "`claude` CLI not installed or not on PATH; install Claude Code or set CAREERAI_CLAUDE_BIN"
    )]
    CliMissing,

    #[error("`claude` CLI session is not authenticated; run `claude login`")]
    CliNotAuthenticated,

    #[error("ANTHROPIC_API_KEY not set in env or keyring")]
    ApiKeyMissing,

    #[error(
        "no LLM backend reachable: `claude` CLI not found AND no ANTHROPIC_API_KEY; \
         install Claude Code (https://claude.ai/download) OR export ANTHROPIC_API_KEY"
    )]
    NoneAvailable,

    #[error("backend `{0}` was forced but its feature is not compiled in")]
    FeatureDisabled(&'static str),

    #[error("llm: {0}")]
    Llm(#[from] LlmError),
}

/// Outcome of [`Backend::probe`] — what the auto-detector picked
/// without actually constructing the driver. Useful for `careerai llm
/// probe` and the plugin setup script.
///
/// `chosen` and `forced` are computed independently — do not infer one
/// from the other:
///
/// * `chosen` reflects strictly what `Backend::resolve(Auto, ...)` would
///   return on this host: claude binary on PATH and ping succeeded
///   (`ClaudeCli`), or an Anthropic API key reachable (`Api`), else
///   `Auto`. It does NOT consider `cfg.backend`.
/// * `forced` records the operator's override from `cfg.backend` (set
///   via `--llm-backend` or `cfg.llm.backend`). Independent of
///   `chosen`; may or may not match.
///
/// The two fields can legitimately disagree without implying the
/// override is broken. For example, with `forced = Some(Api)` and
/// `chosen = ClaudeCli`, auto-detection would pick the CLI, but the
/// operator forced API; that path can still resolve successfully when
/// `api_key_present = true`. Whether the override actually works is
/// answered by calling `Backend::resolve(forced, ...)` — the CLI does
/// this in `probe_forced_resolve` and surfaces a non-zero exit only
/// when that resolve errors. Field comparison alone is not sufficient.
#[derive(Debug, Clone)]
pub struct BackendProbe {
    /// What `Backend::resolve(Auto, ...)` would hand back on this host
    /// (claude binary on PATH + ping ok, or API key reachable).
    /// Strictly auto-detection — does NOT consider `cfg.backend`.
    /// `Auto` means neither backend is reachable.
    pub chosen: BackendChoice,
    /// Operator override from `cfg.backend`, if not `Auto`. Independent
    /// of `chosen` — may legitimately differ when the operator forces a
    /// backend that auto-detection would not have picked. Whether the
    /// override actually resolves is determined by calling
    /// `Backend::resolve(forced, ...)`, not by comparing fields.
    pub forced: Option<BackendChoice>,
    pub claude_binary: Option<PathBuf>,
    pub claude_version: Option<String>,
    pub claude_ping_ms: Option<u128>,
    pub claude_auth_ok: bool,
    /// True if an Anthropic API key is reachable (env or keyring). Set
    /// in tandem with `api_key_source` from a single cached read of
    /// `api_key_source()`.
    pub api_key_present: bool,
    pub api_key_source: Option<&'static str>,
}

impl Backend {
    /// Resolve a backend per the rules in this module's doc.
    ///
    /// `cfg.timeout_seconds` is forwarded to whichever driver wins.
    ///
    /// # Errors
    /// See [`BackendError`].
    pub async fn resolve(
        choice: BackendChoice,
        cfg: &LlmConfig,
        cache: Arc<Cache>,
    ) -> std::result::Result<Self, BackendError> {
        match choice {
            BackendChoice::Auto => resolve_auto(cfg, cache).await,
            BackendChoice::ClaudeCli => build_cli(cfg, cache).await,
            BackendChoice::Api => build_api(cfg, cache),
        }
    }

    pub async fn probe(cfg: &LlmConfig) -> BackendProbe {
        let mut probe = BackendProbe {
            chosen: BackendChoice::Auto,
            forced: None,
            claude_binary: None,
            claude_version: None,
            claude_ping_ms: None,
            claude_auth_ok: false,
            api_key_present: false,
            api_key_source: None,
        };

        #[cfg(feature = "live-llm-cli")]
        {
            if let Ok(bin) = locate_claude_binary() {
                probe.claude_binary = Some(bin.clone());
                if let Some(v) = read_claude_version(&bin).await {
                    probe.claude_version = Some(v);
                }
                // Honor CAREERAI_SKIP_CLI_PROBE for parity with
                // `resolve_auto` and `build_cli`. The module doc claims
                // the env var is honored everywhere; this branch was
                // the lone exception. Skipping the live ping here gives
                // tests and offline scenarios a deterministic "binary
                // present" result without spawning claude.
                let auth_ok = std::env::var("CAREERAI_SKIP_CLI_PROBE").is_ok()
                    || probe_claude_auth(&bin).await;
                if auth_ok {
                    probe.claude_auth_ok = true;
                    probe.chosen = BackendChoice::ClaudeCli;
                    probe.claude_ping_ms = last_ping_ms();
                }
            }
        }

        // Always look for an API key, even when CLI wins, so `probe`
        // output can show the fallback status. Cache the result — each
        // `api_key_source()` call reads env + keyring, and on macOS the
        // keychain query can pop a confirmation dialog. One read per
        // probe.
        let api_source = api_key_source();
        probe.api_key_present = api_source.is_some();
        probe.api_key_source = api_source;
        if probe.api_key_present && probe.chosen == BackendChoice::Auto {
            #[cfg(feature = "live-llm-api")]
            {
                probe.chosen = BackendChoice::Api;
            }
        }

        // Track a forced backend choice from `cfg.backend` separately
        // from the auto-detected `chosen`. Previously `chosen` was
        // overwritten unconditionally with `cfg.backend`, which lied in
        // three cases:
        //   - forced `Api` but `live-llm-api` not compiled
        //   - forced `Api` but no API key
        //   - forced `ClaudeCli` but binary not present / not authed
        // Now `chosen` always reflects what would actually resolve, and
        // `forced` records what the operator asked for. The CLI prints
        // both so the operator can see when their override is unusable.
        if matches!(cfg.backend, BackendChoice::ClaudeCli | BackendChoice::Api) {
            probe.forced = Some(cfg.backend);
        }

        probe
    }
}

async fn resolve_auto(
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    #[cfg(feature = "live-llm-cli")]
    {
        if let Ok(bin) = locate_claude_binary() {
            let auth_ok =
                std::env::var("CAREERAI_SKIP_CLI_PROBE").is_ok() || probe_claude_auth(&bin).await;
            if auth_ok {
                debug!(
                    target = "careerai_llm::backend",
                    binary = %bin.display(),
                    "auto-resolved backend: claude-cli"
                );
                let model = pick_default_model(cfg);
                return Ok(Backend::ClaudeCli(ClaudeCliLlm::new(
                    bin,
                    model,
                    cache,
                    cfg.timeout_seconds.max(1),
                )));
            }
            debug!(
                target = "careerai_llm::backend",
                "claude binary present but ping failed; falling through"
            );
        }
    }

    #[cfg(feature = "live-llm-api")]
    {
        if api_key_source().is_some() {
            return build_api(cfg, cache);
        }
    }

    let _ = cache;
    let _ = cfg;
    Err(BackendError::NoneAvailable)
}

#[cfg(feature = "live-llm-cli")]
async fn build_cli(
    cfg: &LlmConfig,
    cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    let bin = locate_claude_binary().map_err(|e| match e {
        ClaudeCliError::NotInstalled => BackendError::CliMissing,
        other => BackendError::Llm(LlmError::from(other)),
    })?;

    if std::env::var("CAREERAI_SKIP_CLI_PROBE").is_err() && !probe_claude_auth(&bin).await {
        return Err(BackendError::CliNotAuthenticated);
    }

    let model = pick_default_model(cfg);
    Ok(Backend::ClaudeCli(ClaudeCliLlm::new(
        bin,
        model,
        cache,
        cfg.timeout_seconds.max(1),
    )))
}

#[cfg(not(feature = "live-llm-cli"))]
async fn build_cli(
    _cfg: &LlmConfig,
    _cache: Arc<Cache>,
) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("claude-cli"))
}

#[cfg(feature = "live-llm-api")]
fn build_api(cfg: &LlmConfig, cache: Arc<Cache>) -> std::result::Result<Backend, BackendError> {
    let key = read_api_key().ok_or(BackendError::ApiKeyMissing)?;
    let model = pick_default_model(cfg);
    let llm = RigLlm::with_api_key(
        Provider::Anthropic,
        key,
        model,
        cache,
        cfg.timeout_seconds.max(1),
    )
    .map_err(BackendError::Llm)?;
    info!(
        target = "careerai_llm::backend",
        "auto-resolved backend: anthropic-api"
    );
    Ok(Backend::Api(llm))
}

#[cfg(not(feature = "live-llm-api"))]
fn build_api(_cfg: &LlmConfig, _cache: Arc<Cache>) -> std::result::Result<Backend, BackendError> {
    Err(BackendError::FeatureDisabled("api"))
}

/// Pick the default model for the resolved backend. Tailor model wins,
/// then parse_resume_model, then a hard-coded sensible default.
fn pick_default_model(cfg: &LlmConfig) -> String {
    let candidate = if !cfg.tailor_model.is_empty() {
        cfg.tailor_model.as_str()
    } else if !cfg.parse_resume_model.is_empty() {
        cfg.parse_resume_model.as_str()
    } else {
        "sonnet"
    };
    strip_provider_prefix(candidate).to_string()
}

/// Strip a leading `anthropic/` / `openai/` namespace if the layered
/// config used the rig-style multi-provider naming.
fn strip_provider_prefix(model: &str) -> &str {
    if let Some((_, rest)) = model.split_once('/') {
        rest
    } else {
        model
    }
}

/// Best-effort lookup for an Anthropic API key. Mirrors
/// `careerai-cli::anthropic_key_reachable` so behavior is consistent
/// across CLI and library callers.
#[cfg(feature = "live-llm-api")]
fn read_api_key() -> Option<String> {
    if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
        if !v.is_empty() {
            return Some(v);
        }
    }
    keyring::Entry::new("career-ai", "anthropic/api_key")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|v| !v.is_empty())
}

fn api_key_source() -> Option<&'static str> {
    if std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty())
    {
        return Some("env:ANTHROPIC_API_KEY");
    }
    let entry = keyring::Entry::new("career-ai", "anthropic/api_key").ok()?;
    let v = entry.get_password().ok()?;
    if v.is_empty() {
        None
    } else {
        Some("keyring:career-ai/anthropic/api_key")
    }
}

#[cfg(feature = "live-llm-cli")]
async fn read_claude_version(bin: &std::path::Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Two-tier auth probe.
///
/// 1. **Primary** — `claude auth status` (no model call, no token
///    cost, exits in <2s). Parses `loggedIn: true` as authenticated.
/// 2. **Fallback** — `claude --print --output-format json --model
///    sonnet "ping"` with a 60s ceiling. Used when `auth status` is
///    absent (older `claude` builds), exits non-zero, returns
///    unparseable JSON, or omits the `loggedIn` field. The ping path
///    runs a real inference and costs ~$0.08, but stays correct.
///
/// Both paths record elapsed time in `LAST_PING_MS` so the `probe`
/// report still shows latency. Tests bypass everything via
/// `CAREERAI_SKIP_CLI_PROBE=1`.
#[cfg(feature = "live-llm-cli")]
async fn probe_claude_auth(bin: &std::path::Path) -> bool {
    // Try the cheap path first. `claude auth status` exits in <2s on a
    // warm shell and never invokes the model — zero token cost. Older
    // `claude` builds don't ship the subcommand; on any failure mode
    // (non-zero exit, unparseable JSON, missing `loggedIn`, or timeout)
    // fall back to the ping probe.
    if let Some(ok) = probe_claude_auth_status(bin).await {
        return ok;
    }
    probe_claude_auth_via_ping(bin).await
}

/// Cheap auth probe via `claude auth status`. Returns:
///
/// * `Some(true)`  — JSON `{"loggedIn": true, ...}` parsed cleanly.
/// * `Some(false)` — JSON `{"loggedIn": false, ...}` parsed cleanly.
/// * `None`        — `auth status` is unavailable (older claude),
///   exited non-zero, returned unparseable JSON, the `loggedIn` field
///   was missing or non-boolean, or the call timed out. The caller
///   must fall back to the inference-based ping probe.
///
/// Records `LAST_PING_MS` on the success paths so the probe report
/// still shows latency.
#[cfg(feature = "live-llm-cli")]
async fn probe_claude_auth_status(bin: &std::path::Path) -> Option<bool> {
    let started = std::time::Instant::now();
    // 10s is generous for a `claude auth status` call that the user
    // reports completing in <2s. We only need enough headroom for a
    // cold environment where keychain/dbus probes can briefly stall.
    let result = timeout(
        Duration::from_secs(10),
        Command::new(bin).arg("auth").arg("status").output(),
    )
    .await;
    let Ok(Ok(out)) = result else { return None };
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok()?;
    let logged_in = value.get("loggedIn")?.as_bool()?;
    let elapsed = started.elapsed().as_millis();
    LAST_PING_MS.with(|c| c.set(Some(elapsed)));
    Some(logged_in)
}

/// Fallback auth probe used when `claude auth status` is unavailable
/// or unparseable. Invokes `claude --print --output-format json
/// --model sonnet "ping"` with a 60s ceiling — a cold shell can run
/// 14-15s in practice, and the previous 15s cap timed out spuriously.
/// Treats exit-0 + `is_error: false` as success. Note: this path
/// runs a real inference and costs roughly $0.08 per probe; the
/// primary `claude auth status` path is preferred when supported.
#[cfg(feature = "live-llm-cli")]
async fn probe_claude_auth_via_ping(bin: &std::path::Path) -> bool {
    let started = std::time::Instant::now();
    let result = timeout(
        Duration::from_secs(60),
        Command::new(bin)
            .arg("--print")
            .arg("--output-format")
            .arg("json")
            .arg("--model")
            .arg("sonnet")
            .arg("ping")
            .output(),
    )
    .await;
    let Ok(Ok(out)) = result else { return false };
    let elapsed = started.elapsed().as_millis();
    LAST_PING_MS.with(|c| c.set(Some(elapsed)));
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Parse only the field we care about.
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .is_ok_and(|v| v.get("is_error").and_then(serde_json::Value::as_bool) != Some(true))
}

thread_local! {
    static LAST_PING_MS: std::cell::Cell<Option<u128>> = const { std::cell::Cell::new(None) };
}

fn last_ping_ms() -> Option<u128> {
    LAST_PING_MS.with(std::cell::Cell::get)
}

#[async_trait]
impl Llm for Backend {
    fn name(&self) -> &'static str {
        match self {
            #[cfg(feature = "live-llm-cli")]
            Self::ClaudeCli(b) => b.name(),
            #[cfg(feature = "live-llm-api")]
            Self::Api(b) => b.name(),
            #[allow(unreachable_patterns)]
            _ => "none",
        }
    }

    async fn complete(&self, req: &LlmRequest) -> LlmResult<LlmResponse> {
        match self {
            #[cfg(feature = "live-llm-cli")]
            Self::ClaudeCli(b) => b.complete(req).await,
            #[cfg(feature = "live-llm-api")]
            Self::Api(b) => b.complete(req).await,
            #[allow(unreachable_patterns)]
            _ => Err(LlmError::Upstream("no backend feature compiled".into())),
        }
    }
}

#[cfg(test)]
// `clippy::await_holding_lock`: we hold a `std::sync::Mutex` guard
// across `.await` in a few tests. The guard exists purely to serialize
// env-var mutations (CAREERAI_CLAUDE_BIN, ANTHROPIC_API_KEY) across
// parallel cargo-test threads — there's no real contention or deadlock
// risk, and async work inside the guarded section is short.
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::await_holding_lock)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
}
