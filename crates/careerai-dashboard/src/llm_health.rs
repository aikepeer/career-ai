//! Lightweight probe for LLM credential availability.
//!
//! Two-tier check, similar in spirit to `careerai-llm::backend::auth` but
//! kept deliberately minimal so the dashboard stays cheap and avoids
//! pulling in the full LLM crate:
//!
//! 1. `ANTHROPIC_API_KEY` in env → Ready (ApiKey).
//! 2. `claude` binary on PATH → Ready (ClaudeCli).
//! 3. Neither → Missing.
//!
//! Does not shell out to `claude auth status` — that adds ~2s and the
//! dashboard is re-rendered every refresh cycle. Full auth probing belongs
//! in `careerai llm probe`.

use std::time::Duration;

use serde::Serialize;

const WHICH_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LlmHealth {
    Ready,
    Missing,
}

/// Runs both checks concurrently. Env lookups are free; `which claude`
/// has a 500ms timeout so this caps at ~500ms worst case.
pub async fn probe() -> LlmHealth {
    let api_fut = async {
        std::env::var("ANTHROPIC_API_KEY").is_ok() || std::env::var("ANTHROPIC_AUTH_TOKEN").is_ok()
    };
    let cli_fut = async {
        tokio::time::timeout(
            WHICH_TIMEOUT,
            tokio::process::Command::new("which")
                .arg("claude")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .status(),
        )
        .await
        .is_ok_and(|s| s.is_ok_and(|st| st.success()))
    };

    let (has_api_key, has_claude_cli) = tokio::join!(api_fut, cli_fut);
    if has_api_key || has_claude_cli {
        LlmHealth::Ready
    } else {
        LlmHealth::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn probe_returns_some_variant() {
        let h = probe().await;
        assert!(matches!(h, LlmHealth::Ready | LlmHealth::Missing));
    }
}
