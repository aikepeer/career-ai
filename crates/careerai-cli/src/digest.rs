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
use careerai_pipeline as pipeline;

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

/// Build the cookie_warnings list for the digest. Read-only side effect
/// surface: queries the OS keyring, decodes JWT exp claim. Wired here in
/// the CLI layer (rather than inside `pipeline::digest_summary`) so the
/// pipeline crate doesn't have to depend on `careerai-submit` for a
/// purely reporting feature.
fn collect_cookie_warnings() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(remaining) = careerai_submit::credentials::cookie_remaining("linkedin") {
        if remaining < Duration::hours(48) {
            out.push(format!(
                "li_at expires in {} — run `careerai cookies refresh linkedin`",
                humanize(remaining)
            ));
        }
    }
    out
}

/// Run `careerai digest` end to end.
pub async fn run_digest(root: &Path, _cfg: &CoreConfig, since_arg: &str) -> Result<()> {
    let since = parse_since(since_arg)?;
    let mut report = pipeline::digest_summary(root, since).await?;
    // Populate cookie warnings here — pipeline doesn't link careerai-submit.
    report.cookie_warnings = collect_cookie_warnings();

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
}
