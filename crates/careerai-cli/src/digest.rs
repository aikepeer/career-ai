//! `careerai digest` — pretty-print a daily activity summary.
//!
//! Reads pipeline state via `pipeline::digest_summary`, formats it for
//! the operator's terminal. Designed to render in well under one second
//! so it fits cleanly into a daily-review habit (e.g. tmux pane on
//! login, cron-piped to a notification, etc.).

use std::path::Path;

use anyhow::{anyhow, Result};
use chrono::Duration;

use careerai_core::config::CoreConfig;
use careerai_notify::{NotifyEvent, Pipeline as NotifyPipeline, Severity};
use careerai_pipeline as pipeline;
use careerai_submit::credentials::CookieHealth;

/// Parse a since-window string. Accepts `<n>h`, `<n>d`, `<n>w`, or a
/// bare integer interpreted as hours. Returns `chrono::Duration`.
fn parse_since(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty --since value"));
    }
    let (num_part, unit_hours) = match s.chars().last() {
        Some('h') => (&s[..s.len() - 1], 1i64),
        Some('d') => (&s[..s.len() - 1], 24i64),
        Some('w') => (&s[..s.len() - 1], 24 * 7),
        Some(c) if c.is_ascii_digit() => (s, 1i64),
        _ => return Err(anyhow!("unrecognised --since suffix in '{s}' (use h|d|w)")),
    };
    let n: i64 = num_part
        .parse()
        .map_err(|_| anyhow!("--since must be a positive integer; got '{s}'"))?;
    if n <= 0 {
        return Err(anyhow!("--since must be > 0; got '{s}'"));
    }
    Ok(Duration::hours(n.saturating_mul(unit_hours)))
}

/// Format a `chrono::Duration` as a short human-readable string:
/// `"1d 6h"`, `"3h 12m"`, `"45m"`. Used for cookie warnings + cron tick age.
fn humanize(d: Duration) -> String {
    let total_secs = d.num_seconds().max(0);
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let mins = (total_secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

/// Build the cookie_warnings list for the digest. Read-only side
/// effect surface: queries the OS keyring, decodes the JWT `exp`
/// claim. Wired at the CLI layer (rather than inside
/// `pipeline::digest_summary`) so the pipeline crate avoids *calling*
/// keyring code for a purely reporting feature — the `careerai-submit`
/// dependency itself is unconditional but that's only because pipeline
/// owns the apply path; the digest read path stays clean.
fn collect_cookie_warnings_from(health: CookieHealth) -> Vec<String> {
    match health {
        // Cookie is fine — no warning.
        CookieHealth::Healthy(_) => Vec::new(),
        // Cookie absent from keyring entirely.
        CookieHealth::NotStored => {
            vec!["li_at not in keyring — run `careerai cookies refresh linkedin`".to_string()]
        }
        // Present but JWT couldn't be decoded.
        CookieHealth::Unparseable => vec![
            "li_at present in keyring but unparseable — run `careerai cookies refresh linkedin`"
                .to_string(),
        ],
        // Already past exp.
        CookieHealth::Expired(ago) => vec![format!(
            "li_at EXPIRED {} ago — run `careerai cookies refresh linkedin`",
            humanize(ago)
        )],
        // Still valid but inside the 48h warning window.
        CookieHealth::ExpiringSoon(remaining) => vec![format!(
            "li_at expires in {} — run `careerai cookies refresh linkedin`",
            humanize(remaining)
        )],
    }
}

/// Map a `CookieHealth` reading to a `NotifyEvent` + severity, if any.
/// Returns `None` for healthy cookies so the caller can use a single
/// `if let Some(...)` without nested matches.
fn cookie_event_for(provider: &str, health: &CookieHealth) -> Option<(NotifyEvent, Severity)> {
    match health {
        CookieHealth::Healthy(_) => None,
        CookieHealth::ExpiringSoon(remaining) => {
            // Round up so a remaining < 1h still surfaces a non-zero
            // hour count to the operator. `num_hours` returns `i64`;
            // negative values are impossible here (ExpiringSoon means
            // strictly positive) but clamp defensively before casting.
            let hours_left = u64::try_from(remaining.num_hours().max(1)).unwrap_or(0);
            Some((
                NotifyEvent::CookieExpiringSoon {
                    provider: provider.to_string(),
                    hours_left,
                },
                Severity::Warning,
            ))
        }
        CookieHealth::Expired(_) | CookieHealth::NotStored | CookieHealth::Unparseable => Some((
            NotifyEvent::CookieExpiringSoon {
                provider: provider.to_string(),
                hours_left: 0,
            },
            Severity::Critical,
        )),
    }
}

/// Run `careerai digest` end to end.
pub async fn run_digest(root: &Path, cfg: &CoreConfig, since_arg: &str) -> Result<()> {
    let since = parse_since(since_arg)?;
    let mut report = pipeline::digest_summary(root, since).await?;
    // Populate cookie warnings here so `digest_summary` itself stays
    // focused on pipeline state and never calls keyring/submit code.
    // (`careerai-pipeline` links `careerai-submit` for the apply path,
    // but the digest read path stays clean.)
    let health = careerai_submit::credentials::cookie_health("linkedin");
    report.cookie_warnings = collect_cookie_warnings_from(health);

    // Fire a notification if the cookie is unhealthy. Best-effort —
    // `Pipeline::fire` swallows channel errors so this never breaks
    // the digest output.
    if let Some((event, severity)) = cookie_event_for("linkedin", &health) {
        match NotifyPipeline::from_config(&cfg.notify) {
            Ok(pipe) => pipe.fire(event, severity).await,
            Err(e) => tracing::warn!(error = %e, "notify pipeline init failed"),
        }
    }

    println!(
        "career-ai digest — last {} (since {})",
        since_arg, report.since_iso
    );
    println!(
        "  pipeline:  discovered: {}   matched: {}   shortlisted: {}   drafted: {}   submitted: {}   failed: {}",
        report.discovered,
        report.matched,
        report.shortlisted,
        report.drafted,
        report.submitted,
        report.failed,
    );
    println!("  responded: {}", report.responded);

    if report.per_source.is_empty() {
        println!("  sources:   (no activity in window)");
    } else {
        let mut entries: Vec<(&String, &pipeline::SourceCounts)> =
            report.per_source.iter().collect();
        entries.sort_by(|a, b| b.1.total.cmp(&a.1.total).then_with(|| a.0.cmp(b.0)));
        let s = entries
            .iter()
            .map(|(name, c)| format!("{name} {}", c.total))
            .collect::<Vec<_>>()
            .join(" / ");
        println!("  sources:   {s}");
    }

    match &report.last_tick {
        Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
            Ok(dt) => {
                let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
                println!("  last cron tick: {ts} ({} ago)", humanize(age));
            }
            Err(_) => println!("  last cron tick: {ts}"),
        },
        None => println!("  last cron tick: (no events yet — run `careerai discover` once)"),
    }

    if report.cookie_warnings.is_empty() {
        println!("  warnings:  none");
    } else {
        for w in &report.cookie_warnings {
            println!("  warning:   {w}");
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_hours() {
        assert_eq!(parse_since("24h").unwrap(), Duration::hours(24));
        assert_eq!(parse_since("1h").unwrap(), Duration::hours(1));
    }

    #[test]
    fn parse_since_days() {
        assert_eq!(parse_since("7d").unwrap(), Duration::hours(168));
    }

    #[test]
    fn parse_since_weeks() {
        assert_eq!(parse_since("2w").unwrap(), Duration::hours(336));
    }

    #[test]
    fn parse_since_bare_int_is_hours() {
        assert_eq!(parse_since("12").unwrap(), Duration::hours(12));
    }

    #[test]
    fn parse_since_rejects_zero_or_negative() {
        assert!(parse_since("0h").is_err());
        assert!(parse_since("-1h").is_err());
    }

    #[test]
    fn parse_since_rejects_unknown_unit() {
        assert!(parse_since("3y").is_err());
    }

    #[test]
    fn humanize_handles_days_hours_minutes() {
        assert_eq!(humanize(Duration::hours(30)), "1d 6h");
        assert_eq!(humanize(Duration::minutes(45)), "45m");
        assert_eq!(humanize(Duration::seconds(125)), "2m");
    }

    #[test]
    fn humanize_clamps_negative_to_zero() {
        // A future refactor that drops `.max(0)` would leak negative
        // values into the warning string. Pin the contract.
        assert_eq!(humanize(Duration::hours(-5)), "0m");
    }

    #[test]
    fn warnings_silent_for_healthy_cookie() {
        let healthy = CookieHealth::Healthy(Duration::hours(96));
        assert!(collect_cookie_warnings_from(healthy).is_empty());
    }

    #[test]
    fn warnings_fire_inside_48h_quiet_outside() {
        // The whole point of the digest warning is the 48h boundary —
        // a regression flipping this comparator would ship green
        // without a test that pins both sides.
        let inside = CookieHealth::ExpiringSoon(Duration::hours(3));
        let warnings = collect_cookie_warnings_from(inside);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("expires in"));
        // humanize(3h) = "3h 0m" — the duration appears in the message.
        assert!(warnings[0].contains("3h"), "got: {}", warnings[0]);

        // Anything >= 48h is `Healthy` per `cookie_health` and stays
        // silent. (The boundary lives in `cookie_health`, not here.)
        let healthy = CookieHealth::Healthy(Duration::hours(48));
        assert!(collect_cookie_warnings_from(healthy).is_empty());
    }

    #[test]
    fn warnings_split_expired_from_expiring_soon() {
        // Expired cookie must say "EXPIRED ... ago", not "expires in 0m".
        let expired = CookieHealth::Expired(Duration::hours(3));
        let warnings = collect_cookie_warnings_from(expired);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("EXPIRED") && warnings[0].contains("ago"),
            "got: {}",
            warnings[0]
        );
        assert!(!warnings[0].contains("expires in"));
    }

    #[test]
    fn warnings_for_missing_cookie() {
        let warnings = collect_cookie_warnings_from(CookieHealth::NotStored);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("not in keyring"));
        assert!(warnings[0].contains("careerai cookies refresh linkedin"));
    }

    #[test]
    fn warnings_for_unparseable_cookie() {
        let warnings = collect_cookie_warnings_from(CookieHealth::Unparseable);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unparseable"));
        assert!(warnings[0].contains("careerai cookies refresh linkedin"));
    }
}
