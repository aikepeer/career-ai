//! Local copies of `parse_since` and digest-markdown formatting.
//!
//! `careerai-cli::digest` keeps these private and prints to stdout —
//! perfect for the CLI, awkward for an MCP server that needs the
//! markdown as a *value*. We re-implement the small pieces here rather
//! than promote them out of the CLI crate (which would couple the CLI
//! to a public format we may want to evolve).

use anyhow::{anyhow, Result};
use chrono::Duration;

use careerai_pipeline::DigestReport;

/// Parse a `since` window string. Accepts `<n>h`, `<n>d`, `<n>w`, or a
/// bare integer interpreted as hours.
pub(crate) fn parse_since(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty since value"));
    }
    let (num_part, unit_hours) = match s.chars().last() {
        Some('h') => (&s[..s.len() - 1], 1i64),
        Some('d') => (&s[..s.len() - 1], 24i64),
        Some('w') => (&s[..s.len() - 1], 24 * 7),
        Some(c) if c.is_ascii_digit() => (s, 1i64),
        _ => return Err(anyhow!("unrecognised since suffix in '{s}' (use h|d|w)")),
    };
    let n: i64 = num_part
        .parse()
        .map_err(|_| anyhow!("since must be a positive integer; got '{s}'"))?;
    if n <= 0 {
        return Err(anyhow!("since must be > 0; got '{s}'"));
    }
    Ok(Duration::hours(n.saturating_mul(unit_hours)))
}

/// Render a `DigestReport` as a small markdown block. Sorted by source
/// count desc then alpha to match the CLI ordering.
pub(crate) fn format_digest_markdown(since_arg: &str, report: &DigestReport) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let since_iso = &report.since_iso;
    let _ = writeln!(
        out,
        "# career-ai digest — last {since_arg} (since {since_iso})"
    );
    out.push('\n');
    out.push_str("## Pipeline\n\n");
    let _ = writeln!(out, "- discovered: **{}**", report.discovered);
    let _ = writeln!(out, "- matched: **{}**", report.matched);
    let _ = writeln!(out, "- shortlisted: **{}**", report.shortlisted);
    let _ = writeln!(out, "- drafted: **{}**", report.drafted);
    let _ = writeln!(out, "- submitted: **{}**", report.submitted);
    let _ = writeln!(out, "- failed: **{}**", report.failed);
    let _ = writeln!(out, "- responded: **{}**", report.responded);
    out.push('\n');

    out.push_str("## Sources\n\n");
    if report.per_source.is_empty() {
        out.push_str("_no source activity in window_\n\n");
    } else {
        let mut entries: Vec<(&String, &careerai_pipeline::SourceCounts)> =
            report.per_source.iter().collect();
        entries.sort_by(|a, b| b.1.total.cmp(&a.1.total).then_with(|| a.0.cmp(b.0)));
        for (name, counts) in entries {
            let _ = writeln!(out, "- {name}: {}", counts.total);
        }
        out.push('\n');
    }

    out.push_str("## Last cron tick\n\n");
    match &report.last_tick {
        Some(ts) => {
            let _ = writeln!(out, "- {ts}");
        }
        None => {
            out.push_str("- (no events yet — run `careerai discover` once)\n");
        }
    }

    if !report.cookie_warnings.is_empty() {
        out.push_str("\n## Warnings\n\n");
        for w in &report.cookie_warnings {
            let _ = writeln!(out, "- {w}");
        }
    }

    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_hours() {
        assert_eq!(parse_since("3h").unwrap(), Duration::hours(3));
    }

    #[test]
    fn parse_since_days() {
        assert_eq!(parse_since("2d").unwrap(), Duration::hours(48));
    }

    #[test]
    fn parse_since_weeks() {
        assert_eq!(parse_since("1w").unwrap(), Duration::hours(7 * 24));
    }

    #[test]
    fn parse_since_bare_integer_is_hours() {
        assert_eq!(parse_since("12").unwrap(), Duration::hours(12));
    }

    #[test]
    fn parse_since_rejects_zero() {
        assert!(parse_since("0d").is_err());
    }

    #[test]
    fn parse_since_rejects_unknown_suffix() {
        assert!(parse_since("3y").is_err());
    }

    #[test]
    fn parse_since_rejects_empty() {
        assert!(parse_since("").is_err());
    }
}
