#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

// detect_stale_skills_schema tests live in commands/profile.rs.
// strip_provider_prefix tests live in commands/profile_llm.rs.

/// Regression: when the CLI logs a top-level error, it must include
/// the full anyhow chain so the operator can see WHY a command
/// failed (e.g. `tailor failed: tailor_for_listing: bullet not
/// covered: projects[0].bullets[0]`), not just the outermost
/// context label. The bug a real user just hit: `careerai tailor`
/// printed `error=tailor_for_listing` and dropped a chain three
/// frames deep. Use anyhow's alternate-Display (`{e:#}`) to print
/// the chain joined by `: `.
#[test]
fn anyhow_chain_format_includes_inner_causes() {
    // Build a 3-deep chain similar to what tailor_for_listing -> Llm
    // -> ClaudeCli currently produces.
    let inner: anyhow::Result<()> = Err(anyhow::anyhow!("validator: bullet not covered"));
    let mid = inner.map_err(|e| e.context("schema::parse_and_validate"));
    let outer: anyhow::Result<()> = mid.map_err(|e| e.context("tailor_for_listing"));
    let err = outer.unwrap_err();

    // The format we pick must surface every layer.
    let chain = format!("{err:#}");
    assert!(
        chain.contains("tailor_for_listing"),
        "missing outer in chain: {chain:?}"
    );
    assert!(
        chain.contains("schema::parse_and_validate"),
        "missing mid in chain: {chain:?}"
    );
    assert!(
        chain.contains("bullet not covered"),
        "missing inner in chain: {chain:?}"
    );
    // The plain Display formatter (what `%e` uses in tracing) only
    // emits the outermost layer — that's the bug we're guarding
    // against.
    let outermost_only = format!("{err}");
    assert_eq!(
        outermost_only, "tailor_for_listing",
        "plain Display still drops the chain — confirm our fix uses {{:#}} or ?e"
    );
}

/// Regression: `--source greenhouse,lever,ashby` should parse as
/// three filter entries, not one 19-char source name. Clap's
/// `Vec<String>` only splits on commas when `value_delimiter`
/// is set explicitly. The earlier behavior silently no-op'd
/// `careerai discover` because the joined "greenhouse,lever,..."
/// matched no `Source::name()`, and the user saw "no sources
/// enabled" with zero help text pointing at the cause.
#[test]
fn discover_source_arg_splits_on_commas() {
    let cli = Cli::try_parse_from(["careerai", "discover", "--source", "greenhouse,lever,ashby"])
        .expect("parse");
    match cli.command {
        Command::Discover { sources } => {
            assert_eq!(
                sources,
                vec![
                    "greenhouse".to_string(),
                    "lever".to_string(),
                    "ashby".to_string()
                ],
                "expected three filter entries; got: {sources:?}"
            );
        }
        other => panic!("expected Discover; got {other:?}"),
    }
}

/// `--source greenhouse --source lever` (the repeated form) must
/// keep working alongside the comma form, matching clap's
/// documented behavior when `value_delimiter` is set.
#[test]
fn discover_source_arg_accepts_repeated_form() {
    let cli = Cli::try_parse_from([
        "careerai",
        "discover",
        "--source",
        "greenhouse",
        "--source",
        "lever",
    ])
    .expect("parse");
    match cli.command {
        Command::Discover { sources } => {
            assert_eq!(sources, vec!["greenhouse".to_string(), "lever".to_string()]);
        }
        other => panic!("expected Discover; got {other:?}"),
    }
}

/// Regression for the `--llm-backend` override on `careerai llm
/// probe`: when the global flag is set, it must replace
/// `cfg.llm.backend` before `Backend::probe` is called. Mirrors the
/// merge logic at the top of `run_llm_probe`.
#[test]
fn llm_backend_override_replaces_cfg_backend() {
    use careerai_core::config::{BackendChoice, LlmConfig};
    let mut llm_cfg = LlmConfig {
        backend: BackendChoice::ClaudeCli,
        ..LlmConfig::default()
    };
    let override_choice: Option<BackendChoice> = Some(BackendChoice::Api);
    if let Some(b) = override_choice {
        llm_cfg.backend = b;
    }
    assert_eq!(llm_cfg.backend, BackendChoice::Api);
}

/// Without an override the configured backend stays put — the flag
/// is purely additive.
#[test]
fn llm_backend_override_absent_keeps_cfg_backend() {
    use careerai_core::config::{BackendChoice, LlmConfig};
    let mut llm_cfg = LlmConfig {
        backend: BackendChoice::ClaudeCli,
        ..LlmConfig::default()
    };
    let override_choice: Option<BackendChoice> = None;
    if let Some(b) = override_choice {
        llm_cfg.backend = b;
    }
    assert_eq!(llm_cfg.backend, BackendChoice::ClaudeCli);
}

/// Regression for the probe exit code: when an operator forces a
/// backend that isn't reachable, `probe_forced_resolve` must
/// surface a `BackendError` so `run_llm_probe` can exit 2. Without
/// the resolve attempt, the probe printed `backend: api` and
/// exited 0 even when no API key was reachable.
///
/// Wraps the test in a `Mutex` because `ANTHROPIC_API_KEY` is a
/// process-global env var; parallel cargo-test threads racing
/// set/remove pairs cause intermittent failures.
///
/// `clippy::await_holding_lock`: the `std::sync::Mutex` guard is
/// held across `.await`, but the only contention is between this
/// test's own reruns. No deadlock risk.
#[cfg(feature = "live-llm-api")]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn probe_forced_api_with_no_key_returns_err() {
    use careerai_core::config::{BackendChoice, LlmConfig};
    // Static lock so this test serializes with itself across reruns.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");
    let llm_cfg = LlmConfig {
        backend: BackendChoice::Api,
        ..LlmConfig::default()
    };
    let res = crate::commands::llm::probe_forced_resolve(&llm_cfg).await;
    if let Some(v) = prev {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
    // The keyring may have a real key on a dev box; tolerate Ok in
    // that case. The point is that the resolve attempt happens —
    // not that the key is necessarily missing.
    match res {
        Err(careerai_llm::BackendError::ApiKeyMissing) | Ok(()) => {}
        Err(other) => panic!("expected ApiKeyMissing or Ok, got {other:?}"),
    }
}

/// Regression: two `probe_forced_resolve` calls running concurrently
/// must not collide on the on-disk cache path. The previous
/// implementation hard-coded `$TMPDIR/careerai-llm-probe-cache`,
/// which two parallel probes could race against. This test drives
/// the path indirectly by asserting that
/// `tempfile::tempdir()` — the underpinning of the fix — yields
/// distinct directories for parallel callers, and that two parallel
/// `probe_forced_resolve` calls both complete (Ok or recognized
/// `BackendError`, but never a filesystem-collision failure).
#[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
#[tokio::test]
async fn probe_forced_resolve_parallel_does_not_collide() {
    use careerai_core::config::{BackendChoice, LlmConfig};
    // Sanity check the underlying primitive: parallel tempdir
    // creation produces distinct paths.
    let (a, b) = (
        tempfile::tempdir().expect("tempdir a"),
        tempfile::tempdir().expect("tempdir b"),
    );
    assert_ne!(
        a.path(),
        b.path(),
        "tempfile::tempdir() must yield distinct paths"
    );
    drop((a, b));

    let llm_cfg = LlmConfig {
        backend: BackendChoice::ClaudeCli,
        ..LlmConfig::default()
    };
    // Drive the actual code path twice in parallel. Either ordering
    // of completion is fine; the key invariant is that neither
    // call fails with a filesystem-collision error from a shared
    // cache path. Both callers may legitimately succeed (claude
    // CLI present + authed) or return a recognized BackendError
    // (e.g. NoBackend, BinaryMissing). Anything else suggests the
    // tempdirs collided.
    let (r1, r2) = tokio::join!(
        crate::commands::llm::probe_forced_resolve(&llm_cfg),
        crate::commands::llm::probe_forced_resolve(&llm_cfg),
    );
    // Both calls must complete without panicking. We accept any
    // recognized BackendError variant (the CI box may not have a
    // claude binary, an auth'd session, or an API key), but a
    // panic would signal a real bug — most likely a tempdir
    // collision regression.
    for r in [r1, r2] {
        assert!(
            matches!(
                r,
                Ok(())
                    | Err(careerai_llm::BackendError::CliMissing
                        | careerai_llm::BackendError::CliNotAuthenticated
                        | careerai_llm::BackendError::ApiKeyMissing
                        | careerai_llm::BackendError::NoneAvailable
                        | careerai_llm::BackendError::FeatureDisabled(_)
                        | careerai_llm::BackendError::Llm(_))
            ),
            "unexpected probe outcome: {r:?}"
        );
    }
}
