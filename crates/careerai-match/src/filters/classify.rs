use careerai_core::config::{CoreConfig, MatchConfig};
use careerai_sources::RawListing;

use crate::rules::FilterRules;

/// Outcome of running the filters against one listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Reject(&'static str),
}

impl Decision {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

/// Returns `true` if the listing satisfies the `must_include_skills` hard filter.
pub fn apply_must_include_filter(listing: &RawListing, cfg: &MatchConfig) -> bool {
    if cfg.must_include_skills.is_empty() {
        return true;
    }
    let haystack = format!("{} {}", listing.title, listing.description).to_ascii_lowercase();
    cfg.must_include_skills
        .iter()
        .any(|needle| haystack.contains(&needle.to_ascii_lowercase()))
}

pub fn classify(listing: &RawListing, cfg: &CoreConfig, rules: &FilterRules) -> Decision {
    if !apply_must_include_filter(listing, &cfg.matching) {
        return Decision::Reject("missing required skill");
    }

    let title_lc = listing.title.to_lowercase();
    let desc_lc = listing.description.to_lowercase();

    if rules
        .exclude_titles
        .iter()
        .any(|t| title_lc.contains(t.as_str()))
    {
        return Decision::Reject("excluded title");
    }

    if !cfg.user.locations.is_empty() {
        let locations_lc: Vec<String> = cfg
            .user
            .locations
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        if !location_ok(listing, &locations_lc) {
            return Decision::Reject("location not in allowlist");
        }
    }

    if rules
        .exclude_keywords_in_jd
        .iter()
        .any(|k| desc_lc.contains(k.as_str()))
    {
        return Decision::Reject("excluded JD keyword");
    }

    if !rules.require_any_keyword_in_jd.is_empty()
        && !rules
            .require_any_keyword_in_jd
            .iter()
            .any(|k| desc_lc.contains(k.as_str()))
    {
        return Decision::Reject("no required JD keyword");
    }

    if !cfg.domains.is_empty() {
        let domain_kws_lc: Vec<String> = cfg
            .domains
            .iter()
            .flat_map(|d| &d.keywords_any)
            .map(|k| k.to_lowercase())
            .collect();
        if !domain_kws_lc
            .iter()
            .any(|k| title_lc.contains(k.as_str()) || desc_lc.contains(k.as_str()))
        {
            return Decision::Reject("no configured domain keyword in title or JD");
        }
    }

    Decision::Keep
}

fn location_ok(listing: &RawListing, allowlist_lc: &[String]) -> bool {
    if allowlist_lc.is_empty() {
        return true;
    }
    let Some(loc) = listing.location.as_deref() else {
        return allowlist_lc.iter().any(String::is_empty);
    };
    let loc_lc = loc.to_lowercase();
    allowlist_lc
        .iter()
        .any(|allowed| loc_lc.contains(allowed.as_str()))
}
