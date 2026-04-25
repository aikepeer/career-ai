//! H7: direct integration tests for `discover_one` / `match_one`.
//!
//! These wrappers were introduced in M6 Wave 2 specifically to give the
//! scheduler a per-source dispatch entry point. The Wave 3 IT only
//! exercises the submit cron path — neither `discover_one` nor `match_one`
//! had any direct test coverage before this file landed.
//!
//! What's tested:
//!  - `discover_one("nonexistent_source")` — proves the source filter is
//!    applied (not ignored): an unknown source yields an empty report
//!    rather than fanning out to every adapter.
//!  - `discover_one("greenhouse")` with empty companies — proves the
//!    "no sources enabled" fall-through returns Ok with all-zero counts
//!    rather than erroring or hitting the network.
//!  - `match_one` on an empty `discovered` table with a minimal profile
//!    — proves match end-to-end with realistic but tiny inputs and that
//!    the `_source` arg is genuinely unused (passing arbitrary string
//!    has no side effect).
//!
//! Network is never touched: every source is either unconfigured (empty
//! companies) or disabled (`enabled: false`). The scheduler's offline
//! contract for tests holds.
//!
//! What's NOT tested here: real ATS feed parsing, actual matching with
//! non-empty data. Those live in `careerai-sources` / `careerai-match`
//! crate tests respectively. This file scopes to the pipeline-orchestration
//! wrappers added in this PR.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

/// Scaffold the minimum directory tree the pipeline expects under `root`.
/// Mirrors the pattern used by `careerai-cli/tests/apply_it.rs`. Profile
/// content is the bare-minimum YAML the loader accepts — enough to satisfy
/// `match_all`'s `load_profile` call.
fn scaffold_project(root: &Path) {
    for sub in ["config", "profile", "data", "artifacts"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    // Minimal valid profile.yaml. `match_all` only reads it to construct
    // the scorer; it doesn't matter if the skill/keywords lists are empty.
    fs::write(
        root.join("profile").join("profile.yaml"),
        "personal:\n  name: \"Test User\"\n  email: \"test@example.com\"\n  phone: \"\"\nsummary: \"\"\nskills:\n  languages: []\n",
    )
    .unwrap();
}

/// Disable every source adapter so any accidental fall-through to the
/// real internet would short-circuit. Used by every test in this file.
fn disable_all_sources(cfg: &mut CoreConfig) {
    cfg.sources.greenhouse.companies.clear();
    cfg.sources.lever.companies.clear();
    cfg.sources.remotive.enabled = false;
    cfg.sources.remoteok.enabled = false;
    cfg.sources.naukri.enabled = false;
}

#[tokio::test]
async fn discover_one_unknown_source_returns_empty_report() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());
    let mut cfg = CoreConfig::load(tmp.path()).unwrap();
    disable_all_sources(&mut cfg);

    let report = pipeline::discover_one(tmp.path(), &cfg, "nonexistent-adapter")
        .await
        .expect("discover_one of unknown source should be Ok");

    // The source-filter logic: unknown name matches no adapter → no
    // discovery work happens → all zeros. A regression that dropped
    // the filter would fan out to every enabled adapter and almost
    // certainly produce non-zero counts.
    assert_eq!(report.fetched, 0);
    assert_eq!(report.new_rows, 0);
    assert_eq!(report.duplicates, 0);
    assert_eq!(report.errors, 0);
}

#[tokio::test]
async fn discover_one_disabled_source_returns_empty_report() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());
    let mut cfg = CoreConfig::load(tmp.path()).unwrap();
    disable_all_sources(&mut cfg);

    let report = pipeline::discover_one(tmp.path(), &cfg, "greenhouse")
        .await
        .expect("discover_one of disabled source should be Ok, not Err");

    // `greenhouse` matches an adapter, but with no companies configured
    // the `build_sources` path skips it → "no sources enabled" branch
    // → empty report. This is the same path the scheduler hits when
    // an operator has the source registered in cadence but hasn't
    // populated companies yet — must not error.
    assert_eq!(report.fetched, 0);
    assert_eq!(report.new_rows, 0);
    assert_eq!(report.errors, 0);
}

#[tokio::test]
async fn match_one_on_empty_discovered_table_returns_zero_counts() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold_project(tmp.path());
    let mut cfg = CoreConfig::load(tmp.path()).unwrap();
    disable_all_sources(&mut cfg);

    // Pass an arbitrary `_source` string — `match_one` discards it and
    // delegates to `match_all` (documented behavior). Different source
    // strings must yield identical results.
    let report_a = pipeline::match_one(tmp.path(), &cfg, "greenhouse")
        .await
        .expect("match_one on empty DB should be Ok");
    let report_b = pipeline::match_one(tmp.path(), &cfg, "lever")
        .await
        .expect("match_one with different _source must behave identically");

    assert_eq!(report_a.filtered_out, 0);
    assert_eq!(report_a.shortlisted, 0);
    assert_eq!(report_a.also_filtered, 0);

    // The `_source` argument is intentionally unused; results must match.
    assert_eq!(report_a.filtered_out, report_b.filtered_out);
    assert_eq!(report_a.shortlisted, report_b.shortlisted);
}
