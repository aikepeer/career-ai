use careerai_core::config::{LinkedinBrowserFilters, LinkedinBrowserSourceConfig};

use super::parse::{KNOWN_EXPERIENCE_LEVELS, SEARCH_BASE};

/// Build a `linkedin.com/jobs/search/?...` URL for one page.
#[must_use]
pub fn build_search_url(cfg: &LinkedinBrowserSourceConfig, page_idx: u32) -> String {
    let mut q: Vec<(String, String)> = Vec::new();
    if !cfg.keywords.trim().is_empty() {
        q.push(("keywords".to_string(), cfg.keywords.clone()));
    }
    if !cfg.location.trim().is_empty() {
        q.push(("location".to_string(), cfg.location.clone()));
    }
    apply_filter_params(&cfg.filters, &mut q);
    if page_idx > 0 {
        q.push(("start".to_string(), (page_idx * 25).to_string()));
    }
    if q.is_empty() {
        return SEARCH_BASE.to_string();
    }
    let qs = q
        .iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{SEARCH_BASE}?{qs}")
}

fn apply_filter_params(filters: &LinkedinBrowserFilters, out: &mut Vec<(String, String)>) {
    if filters.remote {
        out.push(("f_WT".to_string(), "2".to_string()));
    }
    if let Some(days) = filters.posted_within_days {
        let seconds = u64::from(days) * 86_400;
        out.push(("f_TPR".to_string(), format!("r{seconds}")));
    }
    if !filters.experience_level.is_empty() {
        let codes: Vec<&str> = filters
            .experience_level
            .iter()
            .filter_map(|lvl| {
                let lvl_lc = lvl.to_ascii_lowercase();
                KNOWN_EXPERIENCE_LEVELS
                    .iter()
                    .find(|(name, _)| *name == lvl_lc)
                    .map(|(_, code)| *code)
            })
            .collect();
        if !codes.is_empty() {
            out.push(("f_E".to_string(), codes.join(",")));
        }
    }
}

/// Minimal application/x-www-form-urlencoded encoder.
pub fn urlencode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let safe = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if safe {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}
