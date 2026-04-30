//! Claude CLI auth probe helpers.
//!
//! Two-tier probe: `claude auth status` (cheap, <2s, zero token cost)
//! with fallback to `claude --print "ping"` (real inference, ~$0.08,
//! 60s ceiling).

#[cfg(feature = "live-llm-cli")]
use {std::time::Duration, tokio::process::Command, tokio::time::timeout};

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
/// Both paths record elapsed time in `LAST_PING_MS` on success so
/// the `probe` report shows latency. On total failure (both tiers
/// returned `None`/`false`) `LAST_PING_MS` is left cleared so a
/// stale value from a previous probe on the same thread cannot leak
/// into the new report. Tests bypass everything via
/// `CAREERAI_SKIP_CLI_PROBE=1`.
#[cfg(feature = "live-llm-cli")]
pub(crate) async fn probe_claude_auth(bin: &std::path::Path) -> bool {
    // Clear stale ping_ms from any previous probe on this thread.
    // Both tiers re-set on success; absent that, the report
    // correctly shows ping_ms=None alongside auth_ok=false.
    LAST_PING_MS.with(|c| c.set(None));
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
/// Records `LAST_PING_MS` on the `Some(_)` paths so the probe report
/// still shows latency; leaves it untouched on `None` so the
/// surrounding orchestrator can decide whether to clear or overwrite.
#[cfg(feature = "live-llm-cli")]
async fn probe_claude_auth_status(bin: &std::path::Path) -> Option<bool> {
    let started = std::time::Instant::now();
    // 10s is generous for `claude auth status` (typical <2s). The
    // headroom covers cold environments where keychain/dbus probes
    // briefly stall.
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

pub(crate) fn last_ping_ms() -> Option<u128> {
    LAST_PING_MS.with(std::cell::Cell::get)
}

#[cfg(feature = "live-llm-cli")]
pub(crate) async fn read_claude_version(bin: &std::path::Path) -> Option<String> {
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
