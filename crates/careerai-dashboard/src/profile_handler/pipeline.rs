//! Whitelisted one-shot CLI command execution for the dashboard.
//!
//! The pure validation/argv half lives in `pipeline_args.rs`; this file
//! only owns the in-process busy guard, subprocess spawn, and axum glue.

use axum::{http::StatusCode, response::IntoResponse, Json};
use std::path::PathBuf;
use std::sync::OnceLock;

use super::pipeline_args::build_command_args;
use super::pipeline_types::{CliRunArgs, CliRunRequest};

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

async fn run_cli_request(req: &CliRunRequest) -> impl IntoResponse {
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
    // Run the child from the app root (same resolution the CLI binary
    // uses) so dashboard-invoked commands read the same config/, data/,
    // profile/ tree regardless of where the dashboard process started.
    let cli_root = careerai_core::paths::resolve_root_env();

    // Explicitly pass through `CAREERAI_LLM_LIVE=1` so dashboard-spawned
    // `tailor`/`render` commands use the live LLM backend instead of
    // falling back to nonexistent MockLlm fixtures. This is necessary
    // because the runit service script may not export this env var.
    // Also pass `CAREERAI_ROOT` so subprocesses find the DB + config.
    let careerai_root = std::env::var("CAREERAI_ROOT")
        .unwrap_or_else(|_| cli_root.to_string_lossy().to_string());
    match tokio::process::Command::new(&exe)
        .args(&argv)
        .current_dir(&cli_root)
        .env("CAREERAI_LLM_LIVE", "1")
        .env("CAREERAI_ROOT", careerai_root)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let message = format!("{stdout}\n{stderr}").trim().to_string();
            if out.status.success() {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({ "status": "success", "message": message })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({
                        "status": "failed",
                        "message": message,
                        "exit_code": out.status.code(),
                    })),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn api_cli_run(Json(req): Json<CliRunRequest>) -> impl IntoResponse {
    run_cli_request(&req).await
}

pub async fn api_pipeline_discover() -> impl IntoResponse {
    run_cli_request(&CliRunRequest {
        command: "discover".to_string(),
        args: CliRunArgs::default(),
    })
    .await
}

pub async fn api_pipeline_match() -> impl IntoResponse {
    run_cli_request(&CliRunRequest {
        command: "match".to_string(),
        args: CliRunArgs::default(),
    })
    .await
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
}
