#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::base::Source;
use careerai_core::config::{LinkedinBrowserFilters, LinkedinBrowserSourceConfig};

#[test]
fn unknown_experience_level_dropped_at_construction() {
    let cfg = LinkedinBrowserSourceConfig {
        filters: LinkedinBrowserFilters {
            experience_level: vec!["wizard".to_string(), "mid".to_string()],
            ..LinkedinBrowserFilters::default()
        },
        ..LinkedinBrowserSourceConfig::default()
    };
    let src = LinkedinBrowserSource::new(cfg);
    assert_eq!(src.cfg.filters.experience_level, vec!["mid".to_string()]);
}

#[test]
fn source_name_is_linkedin_browser() {
    let src = LinkedinBrowserSource::new(LinkedinBrowserSourceConfig::default());
    assert_eq!(src.name(), "linkedin-browser");
}

#[test]
fn max_pages_clamped_to_ceiling() {
    let cfg = LinkedinBrowserSourceConfig {
        max_pages: 100,
        ..LinkedinBrowserSourceConfig::default()
    };
    let src = LinkedinBrowserSource::new(cfg);
    assert_eq!(src.cfg.max_pages, MAX_PAGES_CEILING);
}

#[test]
fn max_pages_at_or_below_ceiling_preserved() {
    let cfg = LinkedinBrowserSourceConfig {
        max_pages: 5,
        ..LinkedinBrowserSourceConfig::default()
    };
    let src = LinkedinBrowserSource::new(cfg);
    assert_eq!(src.cfg.max_pages, 5);
}
