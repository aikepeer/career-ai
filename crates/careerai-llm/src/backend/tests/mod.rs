//! Shared scaffolding + entry-point for `Backend` tests. The actual test
//! cases live in sibling files:
//!
//! * `build_tests.rs` — model-picking + forced-resolve + probe-flag tests
//! * `auth_tests.rs`  — two-tier `claude auth status` / ping fallback tests

// `clippy::await_holding_lock`: the `ENV_LOCK` guard is held across
// `.await` in a few tests purely to serialize env-var mutations
// (CAREERAI_CLAUDE_BIN, ANTHROPIC_API_KEY) across parallel cargo-test
// threads — there's no real contention or deadlock risk.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::await_holding_lock)]

use std::sync::Arc;

use careerai_core::config::LlmConfig;

use crate::cache::Cache;

mod auth_tests;
mod build_tests;

pub(super) use crate::ENV_LOCK;

pub(super) fn cfg() -> LlmConfig {
    LlmConfig::default()
}

pub(super) fn cache() -> Arc<Cache> {
    Arc::new(Cache::new(
        std::env::temp_dir().join("careerai-llm-backend-test"),
    ))
}

/// Helper: write an executable shell stub at the given path.
#[cfg(all(unix, feature = "live-llm-cli"))]
pub(super) fn write_stub(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
