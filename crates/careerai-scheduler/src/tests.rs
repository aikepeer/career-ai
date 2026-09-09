#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_core::config::{CoreConfig, McpSourceConfig, McpTransportConfig};

use super::cron::effective_cadence;
use super::Scheduler;

/// Load a fresh `CoreConfig` backed by the embedded defaults — same
/// pattern `careerai-core` uses in its own unit tests. Returns the
/// config plus the tempdir guarding the scratch root, so the dir
/// outlives the test body.
fn embedded_cfg() -> (tempfile::TempDir, CoreConfig) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = CoreConfig::load(tmp.path()).unwrap();
    (tmp, cfg)
}

#[tokio::test]
async fn from_config_with_embedded_defaults_succeeds() {
    let (tmp, cfg) = embedded_cfg();
    // Embedded defaults declare a non-empty cadence map, so this also
    // exercises the per-source registration branch.
    assert!(!cfg.scheduler.cadence.is_empty());
    let sched = Scheduler::from_config(tmp.path(), &cfg).await;
    assert!(sched.is_ok(), "embedded cadence must register cleanly");
}

#[tokio::test]
async fn from_config_with_empty_cadence_succeeds() {
    let (tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    let sched = Scheduler::from_config(tmp.path(), &cfg).await;
    assert!(sched.is_ok(), "empty cadence must not be an error");
}

#[tokio::test]
async fn from_config_skips_invalid_cron_without_failing() {
    // A clearly invalid cron string must not abort startup. Other
    // (valid) sources should still register.
    let (tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler
        .cadence
        .insert("greenhouse".into(), "0 0 */1 * * *".into());
    cfg.scheduler
        .cadence
        .insert("bogus".into(), "this is not a cron expression".into());
    let sched = Scheduler::from_config(tmp.path(), &cfg).await;
    assert!(
        sched.is_ok(),
        "invalid cron in one source must not fail the scheduler",
    );
}

#[tokio::test]
async fn start_then_immediate_shutdown_is_clean() {
    let (tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler
        .cadence
        .insert("greenhouse".into(), "0 0 */1 * * *".into());

    let mut sched = Scheduler::from_config(tmp.path(), &cfg).await.unwrap();
    sched.start().await.unwrap();
    // Use the public shutdown API — same path `run_until_shutdown` uses.
    sched.shutdown().await.expect("shutdown should not error");
}

#[tokio::test]
async fn from_config_with_follow_up_cadence_succeeds() {
    let (tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler.follow_up_cadence = Some("0 0 9 * * *".into());
    let sched = Scheduler::from_config(tmp.path(), &cfg).await;
    assert!(sched.is_ok(), "follow_up_cadence must register cleanly");
}

#[tokio::test]
async fn from_config_without_follow_up_cadence_succeeds() {
    let (tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler.follow_up_cadence = None;
    let sched = Scheduler::from_config(tmp.path(), &cfg).await;
    assert!(sched.is_ok(), "missing follow_up_cadence must not error");
}

/// Helper: minimal enabled MCP source with the given name + cron.
fn mcp_source_with_cron(name: &str, cron: Option<&str>) -> McpSourceConfig {
    McpSourceConfig {
        name: name.to_owned(),
        enabled: true,
        submit_enabled: false,
        cron: cron.map(str::to_owned),
        rate_per_minute: 0,
        mcp: McpTransportConfig {
            command: "/bin/true".to_owned(),
            ..Default::default()
        },
    }
}

#[test]
fn effective_cadence_uses_mcp_per_source_cron_when_set() {
    let (_tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.sources.mcp.clear();
    cfg.sources
        .mcp
        .push(mcp_source_with_cron("linkedin-mcp", Some("0 */2 * * * *")));

    let cadence = effective_cadence(&cfg);
    assert_eq!(
        cadence.get("linkedin-mcp").map(String::as_str),
        Some("0 */2 * * * *"),
        "per-source cron must register when no global cadence entry exists",
    );
}

#[test]
fn effective_cadence_per_source_cron_overrides_global() {
    let (_tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler
        .cadence
        .insert("linkedin-mcp".into(), "0 0 */1 * * *".into());
    cfg.sources.mcp.clear();
    cfg.sources
        .mcp
        .push(mcp_source_with_cron("linkedin-mcp", Some("0 */2 * * * *")));

    let cadence = effective_cadence(&cfg);
    assert_eq!(
        cadence.get("linkedin-mcp").map(String::as_str),
        Some("0 */2 * * * *"),
        "per-source cron must beat the global cadence entry",
    );
}

#[test]
fn effective_cadence_falls_back_to_cadence_map_when_cron_unset() {
    let (_tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler
        .cadence
        .insert("linkedin-mcp".into(), "0 0 */1 * * *".into());
    cfg.sources.mcp.clear();
    cfg.sources
        .mcp
        .push(mcp_source_with_cron("linkedin-mcp", None));

    let cadence = effective_cadence(&cfg);
    assert_eq!(
        cadence.get("linkedin-mcp").map(String::as_str),
        Some("0 0 */1 * * *"),
        "with no per-source cron, the global cadence wins",
    );
}

#[test]
fn effective_cadence_skips_disabled_mcp_sources() {
    let (_tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.sources.mcp.clear();
    let mut s = mcp_source_with_cron("linkedin-mcp", Some("0 */2 * * * *"));
    s.enabled = false;
    cfg.sources.mcp.push(s);

    let cadence = effective_cadence(&cfg);
    assert!(
        cadence.is_empty(),
        "disabled mcp source must not register a cron entry",
    );
}

#[test]
fn disabled_mcp_removes_global_cadence_entry() {
    // Regression: a disabled MCP source must drop any matching
    // `scheduler.cadence` entry, not just skip its own per-source cron.
    // Previously a flipped-off MCP could leave a stale global cron alive.
    let (_tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.scheduler
        .cadence
        .insert("linkedin-jobs".into(), "0 */6 * * *".into());
    cfg.sources.mcp.clear();
    let mut s = mcp_source_with_cron("linkedin-jobs", None);
    s.enabled = false;
    cfg.sources.mcp.push(s);

    let cadence = effective_cadence(&cfg);
    assert!(
        !cadence.contains_key("linkedin-jobs"),
        "disabled mcp source must drop the matching scheduler.cadence entry",
    );
}

#[tokio::test]
async fn from_config_registers_mcp_per_source_cron() {
    // End-to-end check: per-source cron flows all the way into
    // `Scheduler::from_config` and the scheduler builds cleanly.
    let (tmp, mut cfg) = embedded_cfg();
    cfg.scheduler.cadence.clear();
    cfg.sources.mcp.clear();
    cfg.sources
        .mcp
        .push(mcp_source_with_cron("linkedin-mcp", Some("0 */2 * * * *")));

    let sched = Scheduler::from_config(tmp.path(), &cfg).await;
    assert!(sched.is_ok(), "mcp per-source cron must register cleanly");
}
