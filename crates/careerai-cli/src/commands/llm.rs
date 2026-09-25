//! `careerai llm probe` — report which backend (Auto / claude-cli /
//! API) the resolver would pick on this host, plus health detail
//! (binary path, version, ping latency, API-key source).

use anyhow::Result;

use careerai_core::config::CoreConfig;

pub async fn run_probe(
    cfg: &CoreConfig,
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<()> {
    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
    {
        use careerai_core::config::BackendChoice;
        // Honor the global `--llm-backend` flag. Without this, `probe`
        // silently ignored the override even though every other
        // subcommand respects it.
        let mut llm_cfg = cfg.llm.clone();
        if let Some(b) = backend_override {
            llm_cfg.backend = b;
        }
        let probe = careerai_llm::Backend::probe(&llm_cfg).await;

        // Two lines when the operator forced something: their request
        // and what would actually resolve. One line otherwise.
        if let Some(ref forced) = probe.forced {
            println!(
                "active backend:      {} (forced via --llm-backend)",
                forced.as_str()
            );
            println!("auto-detect default: {}", probe.chosen.as_str());
        } else {
            println!(
                "active backend:      {} (auto-detected)",
                probe.chosen.as_str()
            );
        }

        if let Some(bin) = &probe.claude_binary {
            print!("  claude binary: {}", bin.display());
            if let Some(v) = &probe.claude_version {
                print!(" ({v})");
            }
            println!();
            print!("  claude auth:   ");
            if probe.claude_auth_ok {
                if let Some(ms) = probe.claude_ping_ms {
                    println!("ok (ping {ms} ms)");
                } else {
                    println!("ok");
                }
            } else {
                println!("not authenticated; run `claude login`");
            }
        } else {
            println!("  claude binary: not found on PATH");
        }
        match probe.api_key_source {
            Some(src) => println!("  API key:       present ({src})"),
            None => println!("  API key:       not set"),
        }

        // Auto with nothing reachable is hard-fail (exit 1 via anyhow).
        if probe.forced.is_none() && probe.chosen == BackendChoice::Auto {
            anyhow::bail!(
                "no LLM backend reachable; install Claude Code (https://claude.ai/download) \
                 or export ANTHROPIC_API_KEY"
            );
        }

        // Forced override is unusable — try resolving and surface the
        // real error. Exit 2 to distinguish "your override is broken"
        // from "nothing is reachable" (exit 1) so scripts can branch.
        if probe.forced.is_some() {
            if let Err(e) = probe_forced_resolve(&llm_cfg).await {
                eprintln!(
                    "error: forced backend `{}` is unusable: {e}",
                    llm_cfg.backend.as_str()
                );
                std::process::exit(2);
            }
        }

        Ok(())
    }
    #[cfg(not(any(feature = "live-llm-cli", feature = "live-llm-api")))]
    {
        let _ = (cfg, backend_override);
        println!("backend: none (binary built without `live-llm-cli` or `live-llm-api`)");
        Ok(())
    }
}

#[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
pub(crate) async fn probe_forced_resolve(
    llm_cfg: &careerai_core::config::LlmConfig,
) -> std::result::Result<(), careerai_llm::BackendError> {
    use std::sync::Arc;
    // Per-call tempdir so concurrent `careerai llm probe` invocations
    // (e.g. parallel cargo-test threads, or two operators on a shared
    // host) cannot collide on the same on-disk path. The `TempDir`
    // value is held until after `.await` resolves, then `Drop` cleans
    // up the directory automatically.
    let cache_root = tempfile::tempdir()
        .map_err(|e| careerai_llm::BackendError::Llm(careerai_llm::LlmError::Io(e)))?;
    let cache = Arc::new(careerai_llm::Cache::new(cache_root.path().to_path_buf()));
    let res = careerai_llm::Backend::resolve(llm_cfg.backend.clone(), llm_cfg, cache)
        .await
        .map(|_| ());
    drop(cache_root);
    res
}
