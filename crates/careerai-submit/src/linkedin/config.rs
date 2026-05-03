use std::path::PathBuf;

use crate::rate_limiter::RatePolicy;

/// Configuration for `LinkedinSubmitter`. Mirrors the YAML shape under
/// `submit.linkedin.<...>` (NOT a sibling of `submit:` — sub-block).
/// `LinkedinConfig::from_core` lifts a `careerai_core::config::SubmitConfig`
/// into this struct.
#[derive(Debug, Clone)]
pub struct LinkedinConfig {
    /// Where screenshots land. The submitter writes
    /// `<screenshots_dir>/<application_id>-<stage>.png` and embeds the
    /// path into the returned error so an operator can audit the run.
    pub screenshots_dir: PathBuf,
    /// Optional override for the Chromium runtime user-agent. Defaults
    /// to the modern Linux Chrome UA from `BrowserSessionConfig`.
    pub user_agent: Option<String>,
    /// Headless or headed. Default `true`.
    pub headless: bool,
    /// Rate-limit policy. Sane default: 10/day, 120s between, 60s
    /// jitter, 19:00..01:00 UTC quiet hours (favors IST-night
    /// submission).
    pub rate_policy: RatePolicy,
    /// Selector / action timeout in seconds. Default 20s.
    pub action_timeout_seconds: u64,
    /// Inner kill-switch for the final "Submit application" click.
    /// Default `false` — even with `auto_submit=true` and per-source
    /// enabled, the click stays suppressed and the submitter returns
    /// `SourceDisabled` after taking the audit screenshot. M5b will
    /// flip this to true in a deliberate, auditable change.
    pub allow_submit_click: bool,
}

impl Default for LinkedinConfig {
    fn default() -> Self {
        Self {
            screenshots_dir: PathBuf::from("artifacts/screenshots/linkedin"),
            user_agent: None,
            headless: true,
            rate_policy: RatePolicy {
                max_per_day: 10,
                min_seconds_between: 120,
                jitter_seconds: 60,
                quiet_hours_utc: Some((19, 1)),
            },
            action_timeout_seconds: 20,
            allow_submit_click: false,
        }
    }
}

impl LinkedinConfig {
    /// Build a `LinkedinConfig` from the user's `SubmitConfig`.
    #[must_use]
    pub fn from_core(submit_cfg: &careerai_core::config::SubmitConfig) -> Self {
        let lk = submit_cfg.linkedin.clone().validated();
        Self {
            screenshots_dir: lk.screenshots_dir,
            user_agent: lk.user_agent,
            headless: lk.headless,
            rate_policy: RatePolicy {
                max_per_day: lk.max_per_day,
                min_seconds_between: lk.min_seconds_between,
                jitter_seconds: lk.jitter_seconds,
                quiet_hours_utc: lk.quiet_hours_utc,
            },
            action_timeout_seconds: lk.action_timeout_seconds,
            allow_submit_click: lk.allow_submit_click,
        }
    }
}
