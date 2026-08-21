//! Read-only dashboard data assembly.

mod actions;
mod application;
mod config;
mod explorer;

pub use actions::fetch_action_center;
pub use application::fetch_application_detail;
pub use config::fetch_config_view;
pub(crate) use config::{load_core_config, resolve_score_threshold};
pub use explorer::fetch_discovered_explorer;

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

pub async fn fetch_recent_events(pool: &SqlitePool, limit: u32) -> Result<Vec<EventLogItem>> {
    let events = db_queries::list_recent_events(pool, limit, 0).await?;
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
        items.push(EventLogItem {
            id: ev.id,
            listing_id: ev.listing_id,
            from_state: ev.from_state,
            to_state: ev.to_state,
            note: ev.note,
            timestamp: ev.created_at,
            relative_time: relative,
            severity,
        });
    }
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
        items.push(EventLogItem {
            id: ev.id,
            listing_id: ev.listing_id,
            from_state: ev.from_state,
            to_state: ev.to_state,
            note: ev.note,
            timestamp: ev.created_at,
            relative_time: relative,
            severity,
        });
    }
    Ok(items)
}

#[cfg(test)]
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
}
