//! Submission pipeline configuration: global toggles + per-source
//! browser-driven submitter knobs (LinkedIn, Naukri).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SubmitConfig {
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default)]
    pub per_source: HashMap<String, SubmitSource>,
    /// LinkedIn browser submitter knobs (M5a). Only consulted when the
    /// CLI is built with `--features browser`. Additive and `#[serde(default)]`
    /// so pre-M5a `local.yaml` files keep parsing.
    #[serde(default)]
    pub linkedin: LinkedinSubmitConfig,
    /// Naukri.com browser submitter knobs (M5b Tasks 2.5+2.6). Mirrors the
    /// LinkedIn block — all fields have `#[serde(default)]` so a missing
    /// block parses to `NaukriSubmitConfig::default()`.
    #[serde(default)]
    pub naukri: NaukriSubmitConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmitSource {
    #[serde(default)]
    pub enabled: bool,
}

/// LinkedIn-specific browser submitter configuration. Loaded from
/// `submit.linkedin.*` in `config/{default,local}.yaml`. Mirrors the
/// runtime `careerai_submit::LinkedinConfig` one-for-one so the wire-up
/// in `LinkedinConfig::from_core` is a trivial field copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkedinSubmitConfig {
    /// Where the submitter writes pre-submit screenshots. Created on
    /// demand at submit time; relative paths resolve against the
    /// process cwd (typically the repo root).
    pub screenshots_dir: std::path::PathBuf,
    /// Optional Chromium user-agent override. `None` → use
    /// `BrowserSessionConfig` default.
    pub user_agent: Option<String>,
    /// Headless Chromium. Default `true`.
    pub headless: bool,
    /// Rate-limit policy — max submissions per UTC day.
    pub max_per_day: u32,
    /// Minimum seconds between two submissions.
    pub min_seconds_between: u32,
    /// Uniform random jitter (seconds) added after the min-interval
    /// wait to avoid burst fingerprinting.
    pub jitter_seconds: u32,
    /// `[start_hour_utc, end_hour_utc)` window when the limiter refuses
    /// permits. Wraps across midnight (e.g. `[19, 1)` = late IST night).
    /// Both values are validated to be in `0..=23` (start) and `0..=24`
    /// (end inclusive) at config load via `LinkedinSubmitConfig::validated`;
    /// out-of-range values emit a warning and clamp to the default
    /// rather than silently disabling the gate.
    pub quiet_hours_utc: Option<(u32, u32)>,
    /// Per-action selector / click timeout in seconds.
    pub action_timeout_seconds: u64,
    /// Inner kill-switch enforced by `LinkedinSubmitter::run_session`:
    /// even with `auto_submit=true` AND
    /// `per_source.linkedin.enabled=true` AND a valid `li_at` cookie,
    /// the final "Submit application" click stays suppressed unless
    /// this flag is explicitly true. M5a never flips it (the submitter
    /// returns `SourceDisabled` after the audit screenshot); M5b will.
    /// Two locks must be lifted together to ship the click — this flag
    /// AND the M5b code that replaces the inner `SourceDisabled`
    /// return with the actual `element.click()`.
    pub allow_submit_click: bool,
    /// When `true` (default), the daemon never opens a LinkedIn browser
    /// session and never clicks Submit autonomously. After rendering,
    /// LinkedIn applications transition to `Drafted` instead. Operators
    /// confirm submits one at a time via `careerai review`. Flip to
    /// `false` only after weighing the account-restriction risk.
    #[serde(default = "default_interactive_only")]
    pub interactive_only: bool,
}

fn default_interactive_only() -> bool {
    true
}

impl Default for LinkedinSubmitConfig {
    fn default() -> Self {
        Self {
            screenshots_dir: std::path::PathBuf::from("artifacts/screenshots/linkedin"),
            user_agent: None,
            headless: true,
            max_per_day: 10,
            min_seconds_between: 120,
            jitter_seconds: 60,
            // 19:00 UTC = 00:30 IST; runs through 01:00 UTC, covering
            // late IST night when no human is reviewing submissions.
            quiet_hours_utc: Some((19, 1)),
            action_timeout_seconds: 20,
            // Defense-in-depth default OFF — see field doc.
            allow_submit_click: false,
            interactive_only: true,
        }
    }
}

impl LinkedinSubmitConfig {
    /// Range-validate `quiet_hours_utc`. Out-of-range values silently
    /// disabled the quiet-hours gate before, undermining the safety
    /// posture; now we clamp them to the default window and log a
    /// warning. Called from `from_core` in `careerai-submit::linkedin`
    /// so every consumer gets a vetted policy.
    ///
    /// Valid range:
    /// - `start` in `0..=23`, `end` in `0..=24`.
    /// - `start != end` (equal pair is ambiguous: "always quiet" vs
    ///   "never quiet" — either is a footgun, so reject).
    /// - `(0, 24)` is also rejected because `in_window` treats it as
    ///   the wrap-across-midnight pair `[0:00, 24:00)` covering every
    ///   hour — i.e. always quiet, indistinguishable from leaving the
    ///   submitter disabled. Operators who want "never submit" should
    ///   set `submit.per_source.linkedin.enabled = false` instead.
    /// - To disable the quiet-hours gate entirely, set
    ///   `quiet_hours_utc: null` (None).
    #[must_use]
    pub fn validated(mut self) -> Self {
        if let Some((start, end)) = self.quiet_hours_utc {
            let start_ok = start <= 23;
            let end_ok = end <= 24;
            let distinct = start != end;
            // (0, 24) is a degenerate full-coverage wrap — same as
            // "always quiet". Surface it as out-of-range so operators
            // notice they probably wanted `null` or
            // `submit.per_source.linkedin.enabled = false`.
            let not_full_coverage = !(start == 0 && end == 24);
            if !(start_ok && end_ok && distinct && not_full_coverage) {
                tracing::warn!(
                    target: "config",
                    submit_linkedin_quiet_hours = ?(start, end),
                    "out-of-range or full-coverage quiet_hours_utc; clamping to default (19, 1) — \
                     valid range is start in 0..=23, end in 0..=24, start != end, and (0, 24) is reserved \
                     (use `quiet_hours_utc: null` to disable the gate, or `submit.per_source.linkedin.enabled = false`)"
                );
                self.quiet_hours_utc = Some((19, 1));
            }
        }
        self
    }
}

/// Naukri.com browser-driven submitter configuration. Loaded from
/// `submit.naukri.*` in `config/{default,local}.yaml`. Mirrors
/// `LinkedinSubmitConfig` but without the `interactive_only` /
/// `allow_submit_click` LinkedIn-specific gates — Naukri's risk profile
/// is low enough that full daemon-driven auto-submit is acceptable when
/// the operator opts in via `submit.per_source.naukri.enabled=true` and
/// `submit.auto_submit=true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NaukriSubmitConfig {
    /// Where the submitter writes pre-submit screenshots. Relative paths
    /// resolve against the process cwd (typically the repo root).
    pub screenshots_dir: std::path::PathBuf,
    /// Optional Chromium user-agent override. `None` → use
    /// `BrowserSessionConfig` default.
    pub user_agent: Option<String>,
    /// Headless Chromium. Default `true`.
    pub headless: bool,
    /// Rate-limit policy — max submissions per UTC day.
    pub max_per_day: u32,
    /// Minimum seconds between two submissions.
    pub min_seconds_between: u32,
    /// Uniform random jitter (seconds) added after the min-interval
    /// wait to avoid burst fingerprinting.
    pub jitter_seconds: u32,
    /// `[start_hour_utc, end_hour_utc)` window when the limiter refuses
    /// permits. Same semantics as `LinkedinSubmitConfig::quiet_hours_utc`.
    /// Set to `null` to disable the gate.
    pub quiet_hours_utc: Option<(u32, u32)>,
    /// Per-action selector / click timeout in seconds.
    pub action_timeout_seconds: u64,
}

impl NaukriSubmitConfig {
    /// Sanitise quiet-hours config. Same semantics as
    /// `LinkedinSubmitConfig::validated()` — clamps out-of-range or
    /// full-coverage values back to the default `(19, 1)` window with a
    /// warning. Call this before constructing `NaukriConfig` from this
    /// struct so the runtime sees only sane values.
    #[must_use]
    pub fn validated(mut self) -> Self {
        if let Some((start, end)) = self.quiet_hours_utc {
            let start_ok = start <= 23;
            let end_ok = end <= 24;
            let distinct = start != end;
            let not_full_coverage = !(start == 0 && end == 24);
            if !(start_ok && end_ok && distinct && not_full_coverage) {
                tracing::warn!(
                    target: "config",
                    submit_naukri_quiet_hours = ?(start, end),
                    "out-of-range or full-coverage quiet_hours_utc; clamping to default (19, 1) — \
                     valid range is start in 0..=23, end in 0..=24, start != end, and (0, 24) is reserved \
                     (use `quiet_hours_utc: null` to disable the gate, or `submit.per_source.naukri.enabled = false`)"
                );
                self.quiet_hours_utc = Some((19, 1));
            }
        }
        self
    }
}

impl Default for NaukriSubmitConfig {
    fn default() -> Self {
        Self {
            screenshots_dir: std::path::PathBuf::from("artifacts/naukri-audit"),
            user_agent: None,
            headless: true,
            max_per_day: 10,
            min_seconds_between: 60,
            jitter_seconds: 15,
            // 19:00 UTC = 00:30 IST; align with LinkedIn pattern.
            quiet_hours_utc: Some((19, 1)),
            action_timeout_seconds: 30,
        }
    }
}
