//! Read-only dashboard data assembly.

mod actions;
mod application;
mod config;
mod explorer;

pub use actions::fetch_action_center;
pub use application::fetch_application_detail;
pub use config::fetch_config_view;
pub(crate) use config::{load_core_config, resolve_score_threshold};
pub use explorer::{
    fetch_discovered_explorer, fetch_explorer_count, fetch_explorer_filtered,
    fetch_explorer_filtered_count, ExplorerFilter,
};

use careerai_db::queries as db_queries;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::error::Result;
use crate::view::EventLogItem;

fn relative_time_label(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.num_seconds() < 60 {
        "just now".to_string()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}

/// Heuristic for the explorer's "Remote Only" facet. `listings` has no
/// `is_remote` column (the schema stores location text), so we derive the
/// flag from well-known remote markers.
fn is_remote_location(loc: &str) -> bool {
    let l = loc.to_ascii_lowercase();
    [
        "remote",
        "worldwide",
        "anywhere",
        "global",
        "work from home",
        "wfh",
    ]
    .iter()
    .any(|k| l.contains(k))
}

/// Strip VT100 / ANSI escape sequences from terminal log output.
pub fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_escape = false;
    for c in input.chars() {
        if c == '\x1B' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c.is_ascii_alphabetic() || c == 'm' || c == 'K' {
                in_escape = false;
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Parse raw note or command log into a concise headline summary and optional full payload.
pub fn parse_event_note(raw: Option<&str>, default_summary: &str) -> (String, Option<String>) {
    let Some(raw_str) = raw else {
        return (default_summary.to_string(), None);
    };
    let cleaned = strip_ansi_codes(raw_str).trim().to_string();
    if cleaned.is_empty() {
        return (default_summary.to_string(), None);
    }

    let lines: Vec<&str> = cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return (default_summary.to_string(), None);
    }

    let mut key_error = None;
    for line in &lines {
        if let Some(pos) = line.find("ERROR tailor failed error=") {
            let val = line[pos + "ERROR tailor failed error=".len()..]
                .trim_matches('"')
                .to_string();
            key_error = Some(val);
            break;
        } else if let Some(pos) = line.find("live tailor_for_listing:") {
            key_error = Some(line[pos..].to_string());
            break;
        } else if let Some(pos) = line.find("ERROR") {
            key_error = Some(line[pos..].to_string());
            break;
        }
    }

    let summary = if let Some(err) = key_error {
        err
    } else if lines.len() == 1 && lines[0].len() <= 140 {
        lines[0].to_string()
    } else {
        let first = lines[0];
        if first.len() > 140 {
            format!("{}...", &first[..137])
        } else {
            first.to_string()
        }
    };

    let payload = if lines.len() > 1 || cleaned.len() > 140 {
        Some(cleaned)
    } else {
        None
    };

    (summary, payload)
}

/// Fetch recent audit events, merging pipeline state transitions
/// (`events` table) with dashboard-spawned CLI command results
/// (`command_log` table). Both are surfaced in the Events tab so tailor /
/// render / apply failures are visible alongside pipeline transitions.
/// Merged and ordered newest-first.
pub async fn fetch_recent_events(pool: &SqlitePool, limit: u32) -> Result<Vec<EventLogItem>> {
    // Fetch from both tables in parallel, then merge by timestamp.
    let (events_res, commands_res) = tokio::join!(
        db_queries::list_recent_events(pool, limit, 0),
        db_queries::list_recent_commands(pool, limit),
    );

    let mut items = Vec::new();

    if let Ok(events) = events_res {
        for ev in events {
            let relative = relative_time_label(ev.created_at);
            let severity = if ev.to_state == "failed" {
                "error".to_string()
            } else if ev.to_state == "skipped" || ev.from_state.as_deref() == Some("failed") {
                "warn".to_string()
            } else {
                "info".to_string()
            };
            let (summary, raw_payload) =
                parse_event_note(ev.note.as_deref(), "State transition executed");
            items.push(EventLogItem {
                id: ev.id,
                listing_id: ev.listing_id,
                from_state: ev.from_state,
                to_state: ev.to_state,
                note: Some(summary.clone()),
                summary,
                raw_payload,
                timestamp: ev.created_at,
                relative_time: relative,
                severity,
            });
        }
    }

    if let Ok(commands) = commands_res {
        for cmd in commands {
            let relative = relative_time_label(cmd.created_at);
            let severity = match cmd.status.as_str() {
                "failed" | "timeout" | "error" => "error".to_string(),
                _ => "info".to_string(),
            };
            let (summary, raw_payload) =
                parse_event_note(cmd.message.as_deref(), "Command execution finished");
            items.push(EventLogItem {
                id: -cmd.id, // negative to avoid id collision with events table
                listing_id: cmd.listing_id.unwrap_or_default(),
                from_state: None,
                to_state: format!("{} {}", cmd.command, cmd.status),
                note: Some(summary.clone()),
                summary,
                raw_payload,
                timestamp: cmd.created_at,
                relative_time: relative,
                severity,
            });
        }
    }

    // Sort newest-first, then clamp to limit.
    items.sort_by_key(|item| std::cmp::Reverse(item.timestamp));
    items.truncate(limit as usize);

    Ok(items)
}

/// Fetch the audit trail for a single listing, oldest → newest.
pub async fn fetch_events_for_listing(
    pool: &SqlitePool,
    listing_id: &str,
) -> Result<Vec<EventLogItem>> {
    let events = db_queries::events_for(pool, listing_id).await?;
    let mut items = Vec::with_capacity(events.len());
    for ev in events {
        let relative = relative_time_label(ev.created_at);
        let severity = if ev.to_state == "failed" {
            "error".to_string()
        } else if ev.to_state == "skipped" || ev.from_state.as_deref() == Some("failed") {
            "warn".to_string()
        } else {
            "info".to_string()
        };
        let (summary, raw_payload) =
            parse_event_note(ev.note.as_deref(), "State transition executed");
        items.push(EventLogItem {
            id: ev.id,
            listing_id: ev.listing_id,
            from_state: ev.from_state,
            to_state: ev.to_state,
            note: Some(summary.clone()),
            summary,
            raw_payload,
            timestamp: ev.created_at,
            relative_time: relative,
            severity,
        });
    }
    Ok(items)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::is_remote_location;

    #[test]
    fn detects_remote_location_markers() {
        assert!(is_remote_location("Remote"));
        assert!(is_remote_location("Worldwide"));
        assert!(is_remote_location("Remote - India"));
        assert!(!is_remote_location("San Francisco, CA"));
        assert!(!is_remote_location("Delhi / NCR"));
    }

    #[tokio::test]
    async fn fetch_recent_events_merges_command_log() {
        use careerai_core::state::ListingState;
        use careerai_db::models::NewListing;
        use careerai_db::pool::pool_in_memory;
        use careerai_db::queries;
        use careerai_db::queries::{log_command, transition};

        fn fixture(source: &str, ext: &str) -> NewListing {
            NewListing {
                source: source.into(),
                external_id: ext.into(),
                title: "Senior ML Engineer".into(),
                company: "Acme Robotics".into(),
                location: Some("Remote".into()),
                url: format!("https://example.com/{ext}"),
                description: "Build embedded LLM perception systems.".into(),
                raw_json: None,
            }
        }

        let pool = pool_in_memory().await.unwrap();
        let (id, _) = queries::insert_or_ignore(&pool, &fixture("greenhouse", "cl1"))
            .await
            .unwrap();
        transition(&pool, &id, ListingState::Shortlisted, Some("scored"))
            .await
            .unwrap();

        // Log a failed tailor — should surface as an error-severity event.
        log_command(
            &pool,
            "tailor",
            Some(&id),
            "failed",
            Some(1),
            Some("LLM backend error"),
        )
        .await
        .unwrap();

        // Log a successful discover.
        log_command(&pool, "discover", None, "success", Some(0), None)
            .await
            .unwrap();

        let items = super::fetch_recent_events(&pool, 50).await.unwrap();

        // 1 discovered event from insert + 1 transition + 2 command-log entries = 4 items.
        assert_eq!(items.len(), 4);

        // The failed tailor must be present with error severity.
        let failed = items
            .iter()
            .find(|e| e.to_state.contains("tailor") && e.to_state.contains("failed"))
            .expect("failed tailor event should be in merged feed");
        assert_eq!(failed.severity, "error");
        assert_eq!(failed.listing_id, id);
        assert!(failed.note.as_deref().unwrap().contains("LLM backend"));

        // The successful discover must be present with info severity.
        let discover = items
            .iter()
            .find(|e| e.to_state.contains("discover"))
            .expect("discover event should be in merged feed");
        assert_eq!(discover.severity, "info");
    }

    #[test]
    fn test_strip_ansi_and_parse_event_note() {
        let raw = "\x1b[2m2026-08-30T17:29:04Z\x1b[0m \x1b[32m INFO\x1b[0m db: migrations applied\n\x1b[31mERROR\x1b[0m tailor failed error=\"live tailor_for_listing: invented content at experience[0].bullets[1]: reason=invented proper noun; offending_token=\\\"High\\\"\"";
        let (summary, payload) = super::parse_event_note(Some(raw), "default");
        assert!(summary.contains("live tailor_for_listing: invented content"));
        assert!(summary.contains("offending_token"));
        assert!(summary.contains("High"));
        assert!(payload.is_some());
        let p = payload.unwrap();
        assert!(!p.contains("\x1b["));
        assert!(p.contains("migrations applied"));
    }
}
