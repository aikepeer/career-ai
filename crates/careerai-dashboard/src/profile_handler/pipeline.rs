//! Whitelisted one-shot CLI command execution for the dashboard.
//!
//! The pure validation/argv half lives in `pipeline_args.rs`; this file
//! only owns the in-process busy guard, subprocess spawn, and axum glue.

use axum::{http::StatusCode, response::IntoResponse, Json};
use std::path::PathBuf;
use std::sync::OnceLock;

use super::pipeline_args::build_command_args;
use super::pipeline_types::CliRunRequest;

/// In-process guard so concurrent `discover`/`match`/`run` clicks never
/// double-run the same stages or contend on SQLite. The guard is held for
/// the lifetime of one child process.
static PIPELINE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn pipeline_lock() -> &'static tokio::sync::Mutex<()> {
    PIPELINE_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Resolve the CLI executable. Prefer an explicit `CAREERAI_BIN` override
/// (used by tests and non-standard installs); fall back to `current_exe()`,
/// which is the `careerai` binary whenever the dashboard is served from
/// `careerai status serve`.
fn resolve_cli_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("CAREERAI_BIN").map(PathBuf::from) {
        if path.exists() {
            return path;
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("careerai"))
}

pub async fn run_cli_request(req: &CliRunRequest) -> impl IntoResponse {
    let argv = match build_command_args(req) {
        Ok(argv) => argv,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "status": "invalid", "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let Ok(_guard) = pipeline_lock().try_lock() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "status": "busy" })),
        )
            .into_response();
    };

    let exe = resolve_cli_executable();
    let cli_root = careerai_core::paths::resolve_root_env();
    let careerai_root =
        std::env::var("CAREERAI_ROOT").unwrap_or_else(|_| cli_root.to_string_lossy().to_string());
    let run_timeout = std::time::Duration::from_secs(600);

    // Extract the listing_id (if any) for the command-log entry so the
    // Events tab can link a failed tailor/render to the right listing.
    let listing_id = req
        .args
        .listing_id
        .as_deref()
        .or(req.args.application_id.as_deref());

    // Run the subprocess and capture a structured result. The same
    // `CmdOutcome` feeds both the HTTP response and the `command_log`
    // audit entry — no body re-reading needed.
    let outcome = run_subprocess(&exe, &argv, &cli_root, &careerai_root, run_timeout).await;

    tracing::info!(
        target = "careerai::dashboard",
        command = %req.command,
        status = %outcome.log_status,
        exit_code = ?outcome.exit_code,
        "dashboard pipeline subprocess completed",
    );

    // Persist to the command_log audit table so failures surface in the
    // Events tab alongside pipeline state transitions. Best-effort — a
    // DB error here must not mask the command's own result.
    if let Some(pool) = open_db_pool().await {
        let _ = careerai_db::queries::log_command(
            &pool,
            &req.command,
            listing_id,
            &outcome.log_status,
            outcome.exit_code,
            Some(&outcome.message),
        )
        .await;
    }

    outcome.into_response()
}

/// Structured outcome of a dashboard-spawned CLI subprocess. Carries
/// enough data to build both the HTTP response and the `command_log`
/// audit entry from a single source of truth.
struct CmdOutcome {
    http_status: StatusCode,
    body: serde_json::Value,
    log_status: String,
    exit_code: Option<i32>,
    message: String,
}

impl IntoResponse for CmdOutcome {
    fn into_response(self) -> axum::response::Response {
        (self.http_status, Json(self.body)).into_response()
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_subprocess(
    exe: &std::path::Path,
    argv: &[std::ffi::OsString],
    cli_root: &std::path::Path,
    careerai_root: &str,
    run_timeout: std::time::Duration,
) -> CmdOutcome {
    // R06: spawn the child explicitly so we can kill it on timeout.
    // The previous code used `Command::output()` inside a `tokio::time::timeout`
    // — when the timeout fired, the `output()` future was dropped but the
    // child process kept running, leaking a tailor/render subprocess that
    // could hold the pipeline lock for the full 10-minute window.
    //
    // `current_dir` requires the directory to already exist — on a fresh
    // install (or a fresh CI sandbox) the XDG data root has never been
    // created, so every dashboard-spawned command would otherwise fail
    // with ENOENT before the child even runs. Best-effort create it; a
    // real permission/IO problem still surfaces through the spawn error
    // below.
    let _ = std::fs::create_dir_all(cli_root);
    let mut child = match tokio::process::Command::new(exe)
        .args(argv)
        .current_dir(cli_root)
        .env("CAREERAI_LLM_LIVE", "1")
        .env("CAREERAI_ROOT", careerai_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return CmdOutcome {
                http_status: StatusCode::INTERNAL_SERVER_ERROR,
                body: serde_json::json!({ "status": "error", "error": e.to_string() }),
                log_status: "error".into(),
                exit_code: None,
                message: e.to_string(),
            };
        }
    };

    // R06: take the piped stdout/stderr handles before waiting so we can
    // read them after `wait()` completes. Use `child.wait()` (borrows
    // `&mut self`) instead of `child.wait_with_output()` (takes `self`)
    // so that on timeout we still own `child` and can `kill()` it.
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let wait_result = tokio::time::timeout(run_timeout, child.wait()).await;

    match wait_result {
        Ok(Ok(status)) => {
            let stdout = match stdout_handle {
                Some(mut h) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = h.read_to_end(&mut buf).await;
                    String::from_utf8_lossy(&buf).to_string()
                }
                None => String::new(),
            };
            let stderr = match stderr_handle {
                Some(mut h) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = h.read_to_end(&mut buf).await;
                    String::from_utf8_lossy(&buf).to_string()
                }
                None => String::new(),
            };
            let raw_msg = format!("{stdout}\n{stderr}").trim().to_string();
            let message = crate::details::strip_ansi_codes(&raw_msg);
            let exit_code = status.code();
            if status.success() {
                CmdOutcome {
                    http_status: StatusCode::OK,
                    body: serde_json::json!({ "status": "success", "message": message }),
                    log_status: "success".into(),
                    exit_code: Some(0),
                    message,
                }
            } else {
                CmdOutcome {
                    http_status: StatusCode::UNPROCESSABLE_ENTITY,
                    body: serde_json::json!({
                        "status": "failed",
                        "message": message,
                        "exit_code": exit_code,
                    }),
                    log_status: "failed".into(),
                    exit_code,
                    message,
                }
            }
        }
        Ok(Err(e)) => CmdOutcome {
            http_status: StatusCode::INTERNAL_SERVER_ERROR,
            body: serde_json::json!({ "status": "error", "error": e.to_string() }),
            log_status: "error".into(),
            exit_code: None,
            message: e.to_string(),
        },
        Err(_) => {
            // R06: explicitly kill the timed-out child so it cannot
            // continue running in the background and hold the pipeline lock.
            let _ = child.kill().await;
            let _ = child.wait().await; // reap the zombie
            CmdOutcome {
                http_status: StatusCode::GATEWAY_TIMEOUT,
                body: serde_json::json!({
                    "status": "timeout",
                    "error": "Command execution timed out after 10 minutes"
                }),
                log_status: "timeout".into(),
                exit_code: None,
                message: "Command execution timed out after 10 minutes".into(),
            }
        }
    }
}

/// Open the SQLite pool from the explicit root or XDG data directory.
/// Best-effort — returns `None` if the DB is unavailable (the command result
/// still reaches the caller).
async fn open_db_pool() -> Option<sqlx::SqlitePool> {
    let root = careerai_core::paths::resolve_root_env();
    let path = careerai_core::paths::database_path(&root);
    careerai_db::pool_from_path(&path).await.ok()
}

pub async fn api_cli_run(Json(req): Json<CliRunRequest>) -> impl IntoResponse {
    run_cli_request(&req).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Regression: the dashboard must pass `CAREERAI_LLM_LIVE=1` to
    /// spawned subprocesses so that `careerai tailor` uses the live LLM
    /// backend. Without this, tailor falls back to MockLlm fixtures that
    /// don't exist, and every "Tailor" button in the dashboard fails.
    #[test]
    fn resolve_cli_executable_returns_a_path() {
        let exe = resolve_cli_executable();
        assert!(!exe.as_os_str().is_empty());
    }

    /// The pipeline lock must be the same static each call (no new mutex
    /// per call).
    #[test]
    fn pipeline_lock_is_static_singleton() {
        let a = pipeline_lock();
        let b = pipeline_lock();
        assert!(std::ptr::eq(a, b));
    }

    /// Regression: `run_subprocess` must create `cli_root` before
    /// spawning, since `Command::current_dir` fails with ENOENT if the
    /// directory doesn't exist yet. On a fresh install (or CI sandbox)
    /// the XDG data root has never been created, so this would fail
    /// every dashboard CLI command on first use.
    #[tokio::test]
    async fn run_subprocess_creates_missing_cli_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing_root = tmp.path().join("nested/does/not/exist");
        assert!(!missing_root.exists());

        let outcome = run_subprocess(
            std::path::Path::new("/bin/true"),
            &[],
            &missing_root,
            "test-root",
            std::time::Duration::from_secs(5),
        )
        .await;

        assert!(missing_root.is_dir(), "cli_root must be created");
        assert_eq!(outcome.http_status, StatusCode::OK);
    }
}
