//! Known-source catalog + submit-gate display resolution.

const KNOWN_SOURCES: &[(&str, &str, &str)] = &[
    (
        "greenhouse",
        "ATS Direct Feed",
        "https://boards.greenhouse.io",
    ),
    ("lever", "ATS Direct Feed", "https://jobs.lever.co"),
    ("ashby", "ATS Direct Feed", "https://jobs.ashbyhq.com"),
    (
        "teamtailor",
        "ATS Direct Feed",
        "https://www.teamtailor.com",
    ),
    ("workday", "ATS Direct Feed", "https://myworkdayjobs.com"),
    (
        "smartrecruiters",
        "ATS Direct Feed",
        "https://careers.smartrecruiters.com",
    ),
    (
        "linkedin",
        "Web Scraper / CDP",
        "https://www.linkedin.com/jobs",
    ),
    ("indeed", "Job Board", "https://www.indeed.com"),
    ("naukri", "India Job Portal", "https://www.naukri.com"),
    ("remotive", "Remote Jobs API", "https://remotive.com"),
    (
        "wellfound",
        "Startup Tech Jobs",
        "https://wellfound.com/jobs",
    ),
    (
        "weworkremotely",
        "Remote Community",
        "https://weworkremotely.com",
    ),
    (
        "ycombinator",
        "YC Startups",
        "https://www.workatastartup.com",
    ),
    ("upwork", "Freelance Platform", "https://www.upwork.com"),
    (
        "freelancer",
        "Freelance Platform",
        "https://www.freelancer.com",
    ),
    ("toptal", "Elite Freelance", "https://www.toptal.com"),
    ("remoteok", "Remote Tech Board", "https://remoteok.com"),
    ("freehire", "Aggregator REST API", "https://freehire.me"),
    ("otta", "Curated Tech Jobs", "https://otta.com"),
];

/// Case-insensitive membership test for the fixed known-source catalog.
/// Used by the CLI dispatcher to reject arbitrary `--source` values.
pub(crate) fn is_known_source(name: &str) -> bool {
    KNOWN_SOURCES
        .iter()
        .any(|(n, _, _)| n.eq_ignore_ascii_case(name))
}

pub fn build_sources_list(
    db_rows: Vec<(String, i64, Option<chrono::DateTime<chrono::Utc>>)>,
    core_cfg: Option<&careerai_core::config::CoreConfig>,
) -> Vec<crate::view::ConfigSourceItem> {
    let mut db_map = std::collections::HashMap::new();
    for (name, count, last_sync) in db_rows {
        if !name.is_empty() {
            db_map.insert(name.to_lowercase(), (count, last_sync));
        }
    }

    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for &(s_name, s_kind, s_url) in KNOWN_SOURCES {
        let name_lower = s_name.to_lowercase();
        seen.insert(name_lower.clone());
        let (count_i, last_sync) = db_map.get(&name_lower).copied().unwrap_or((0, None));
        let sync_label = last_sync.map_or_else(
            || "Ready".into(),
            |ts| {
                let delta = chrono::Utc::now().signed_duration_since(ts);
                if delta.num_hours() < 1 {
                    "just now".into()
                } else if delta.num_hours() < 24 {
                    format!("{}h ago", delta.num_hours())
                } else {
                    format!("{}d ago", delta.num_days())
                }
            },
        );
        let status = if count_i > 0 { "active" } else { "ready" }.to_string();
        let (submit_enabled, max_per_day, min_seconds_between, quiet_hours_utc) =
            core_cfg.map_or((false, 0, 0, None), |c| submit_gate_for(c, s_name));

        sources.push(crate::view::ConfigSourceItem {
            name: s_name.to_string(),
            kind: s_kind.to_string(),
            listing_count: u64::try_from(count_i.max(0)).unwrap_or(0),
            last_sync,
            last_sync_label: sync_label,
            status,
            url: Some(s_url.to_string()),
            submit_enabled,
            max_per_day,
            min_seconds_between,
            quiet_hours_utc,
        });
    }

    for (name, (count_i, last_sync)) in db_map {
        if !seen.contains(&name) {
            let (submit_enabled, max_per_day, min_seconds_between, quiet_hours_utc) =
                core_cfg.map_or((false, 0, 0, None), |c| submit_gate_for(c, &name));
            sources.push(crate::view::ConfigSourceItem {
                name: name.clone(),
                kind: "Custom Portal".into(),
                listing_count: u64::try_from(count_i.max(0)).unwrap_or(0),
                last_sync,
                last_sync_label: "active".into(),
                status: "active".into(),
                url: None,
                submit_enabled,
                max_per_day,
                min_seconds_between,
                quiet_hours_utc,
            });
        }
    }

    sources
}

/// Surface the submit gate for one source: the `enabled` flag plus the
/// resolved rate policy (per-source submit block → browser block → legacy
/// `rates.*` → default). Purely a display helper; enforcement lives in
/// `careerai-submit`.
fn submit_gate_for(
    cfg: &careerai_core::config::CoreConfig,
    name: &str,
) -> (bool, u32, u32, Option<(u32, u32)>) {
    let enabled = cfg.submit.per_source.get(name).is_some_and(|s| s.enabled);

    if let Some(ss) = cfg.submit.per_source.get(name) {
        if ss.max_per_day != 0 || ss.min_seconds_between != 0 || ss.quiet_hours_utc.is_some() {
            return (
                enabled,
                ss.max_per_day,
                ss.min_seconds_between,
                ss.quiet_hours_utc,
            );
        }
    }

    if name == "linkedin" {
        let l = &cfg.submit.linkedin;
        return (
            enabled,
            l.max_per_day,
            l.min_seconds_between,
            l.quiet_hours_utc,
        );
    }
    if name == "naukri" {
        let n = &cfg.submit.naukri;
        return (
            enabled,
            n.max_per_day,
            n.min_seconds_between,
            n.quiet_hours_utc,
        );
    }

    if let Some(sr) = cfg
        .rates
        .per_source
        .get(name)
        .or_else(|| cfg.rates.per_source.get("ats_http"))
    {
        let quiet = match sr.quiet_hours.as_slice() {
            [s, e] => Some((*s, *e)),
            _ => None,
        };
        return (enabled, sr.max_per_day, sr.min_seconds_between, quiet);
    }

    (enabled, 0, 0, None)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn freehire_is_a_known_discovery_source() {
        assert!(is_known_source("freehire"));
        assert!(is_known_source("FREEHIRE"));
    }

    #[test]
    fn unknown_sources_are_rejected() {
        assert!(!is_known_source("totally-made-up-board"));
    }

    #[test]
    fn submit_gate_for_per_source_block_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = careerai_core::config::CoreConfig::load(tmp.path()).unwrap();
        cfg.submit.per_source.insert(
            "greenhouse".to_string(),
            careerai_core::config::SubmitSource {
                enabled: true,
                max_per_day: 9,
                min_seconds_between: 30,
                jitter_seconds: 0,
                quiet_hours_utc: Some((1, 2)),
            },
        );

        assert_eq!(
            submit_gate_for(&cfg, "greenhouse"),
            (true, 9, 30, Some((1, 2)))
        );
    }

    #[test]
    fn submit_gate_for_falls_back_to_legacy_rates() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = careerai_core::config::CoreConfig::load(tmp.path()).unwrap();

        // `indeed` has a legacy `rates.indeed` block and an explicit
        // `submit.per_source.indeed.enabled=false`, but no rate fields in
        // its per-source submit block.
        assert_eq!(submit_gate_for(&cfg, "indeed"), (false, 15, 90, None));
    }

    #[test]
    fn submit_gate_for_linkedin_uses_browser_block() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = careerai_core::config::CoreConfig::load(tmp.path()).unwrap();
        cfg.submit.per_source.insert(
            "linkedin".to_string(),
            careerai_core::config::SubmitSource {
                enabled: true,
                ..Default::default()
            },
        );
        cfg.submit.linkedin.max_per_day = 3;
        cfg.submit.linkedin.min_seconds_between = 120;
        cfg.submit.linkedin.quiet_hours_utc = Some((19, 1));

        assert_eq!(
            submit_gate_for(&cfg, "linkedin"),
            (true, 3, 120, Some((19, 1)))
        );
    }

    #[test]
    fn submit_gate_for_unknown_source_falls_back_to_ats_http() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = careerai_core::config::CoreConfig::load(tmp.path()).unwrap();

        // `lever` has `submit.per_source.lever.enabled=true` and no
        // per-source rates; the legacy `rates.ats_http` ceiling applies.
        assert_eq!(submit_gate_for(&cfg, "lever"), (true, 50, 0, None));
    }

    #[test]
    fn submit_gate_for_no_policy_defaults_to_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = careerai_core::config::CoreConfig::load(tmp.path()).unwrap();
        cfg.rates.per_source.clear();

        assert_eq!(submit_gate_for(&cfg, "mystery"), (false, 0, 0, None));
    }
}
