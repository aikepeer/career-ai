//! Rate policy resolution and ATS source classification.

use crate::rate_limiter::RatePolicy;
use careerai_core::config::{RatesConfig, SubmitConfig};

/// True when `source` routes to a non-browser ATS HTTP submitter.
/// Browser submitters (`linkedin`, `naukri`) manage their own rate
/// policy, so they are excluded from the boundary-level acquisition.
pub fn is_ats_http_source(source: &str) -> bool {
    matches!(
        source,
        "greenhouse" | "lever" | "ashby" | "teamtailor" | "smartrecruiters"
    )
}

/// Resolve the rate policy for an ATS HTTP source from
/// `submit.per_source.<source>.*` (the same block that carries the
/// `enabled` gate). Returns `None` when the block has no explicit rate
/// fields so the caller can fall back to the legacy `config.rates`
/// entries without silently overriding them.
pub fn submit_source_rate(submit: &SubmitConfig, source: &str) -> Option<RatePolicy> {
    let ss = submit.per_source.get(source)?;
    let has_explicit_rate = ss.max_per_day != 0
        || ss.min_seconds_between != 0
        || ss.jitter_seconds != 0
        || ss.quiet_hours_utc.is_some();
    if !has_explicit_rate {
        return None;
    }

    let default = RatePolicy::default();
    let quiet_hours_utc = ss.quiet_hours_utc.filter(|(start, end)| {
        *start <= 23 && *end <= 24 && start != end && !(*start == 0 && *end == 24)
    });
    Some(RatePolicy {
        max_per_day: if ss.max_per_day == 0 {
            default.max_per_day
        } else {
            ss.max_per_day
        },
        min_seconds_between: ss.min_seconds_between,
        jitter_seconds: ss.jitter_seconds,
        quiet_hours_utc,
    })
}

/// Resolve the rate policy for an ATS HTTP source from `config.rates`.
///
/// Lookup order: the source's own `rates.<source>` entry, then the shared
/// `rates.ats_http` group entry, then a conservative default. A
/// `max_per_day` of `0` is treated as "unset" and replaced with the
/// conservative default so an incomplete override cannot silently zero
/// out the cap.
pub fn rate_policy_for(rates: &RatesConfig, source: &str) -> RatePolicy {
    let Some(sr) = rates
        .per_source
        .get(source)
        .or_else(|| rates.per_source.get("ats_http"))
    else {
        return RatePolicy::default();
    };

    let default = RatePolicy::default();
    RatePolicy {
        max_per_day: if sr.max_per_day == 0 {
            default.max_per_day
        } else {
            sr.max_per_day
        },
        min_seconds_between: sr.min_seconds_between,
        jitter_seconds: sr.jitter_seconds,
        quiet_hours_utc: match sr.quiet_hours.as_slice() {
            [start, end]
                if *start <= 23 && *end <= 24 && start != end && !(*start == 0 && *end == 24) =>
            {
                Some((*start, *end))
            }
            _ => None,
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::HashMap;

    use careerai_core::config::{RatesConfig, SourceRate, SubmitConfig, SubmitSource};

    use super::{is_ats_http_source, rate_policy_for, submit_source_rate};

    fn rate(max_per_day: u32, min_seconds_between: u32, jitter_seconds: u32) -> SourceRate {
        SourceRate {
            max_per_day,
            min_seconds_between,
            jitter_seconds,
            quiet_hours: Vec::new(),
        }
    }

    fn rates(entries: &[(&str, SourceRate)]) -> RatesConfig {
        RatesConfig {
            per_source: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<HashMap<_, _>>(),
        }
    }

    #[test]
    fn is_ats_http_source_distinguishes_http_from_browser() {
        for src in [
            "greenhouse",
            "lever",
            "ashby",
            "teamtailor",
            "smartrecruiters",
        ] {
            assert!(is_ats_http_source(src), "{src} should be ATS HTTP");
        }
        for src in ["linkedin", "naukri", "remotive", "remoteok", "indeed"] {
            assert!(!is_ats_http_source(src), "{src} should not be ATS HTTP");
        }
    }

    #[test]
    fn rate_policy_for_falls_back_to_ats_http_group() {
        let mut group = rate(7, 30, 5);
        group.quiet_hours = vec![0, 7];
        let cfg = rates(&[("ats_http", group)]);
        let p = rate_policy_for(&cfg, "greenhouse");
        assert_eq!(p.max_per_day, 7);
        assert_eq!(p.min_seconds_between, 30);
        assert_eq!(p.jitter_seconds, 5);
        assert_eq!(p.quiet_hours_utc, Some((0, 7)));
    }

    #[test]
    fn rate_policy_for_prefers_source_over_group() {
        let cfg = rates(&[
            ("ats_http", rate(50, 60, 30)),
            ("greenhouse", rate(3, 0, 0)),
        ]);
        let p = rate_policy_for(&cfg, "greenhouse");
        assert_eq!(p.max_per_day, 3);
        assert_eq!(p.min_seconds_between, 0);
    }

    #[test]
    fn submit_source_rate_parses_explicit_fields() {
        let mut submit = SubmitConfig::default();
        submit.per_source.insert(
            "greenhouse".into(),
            SubmitSource {
                enabled: true,
                max_per_day: 15,
                min_seconds_between: 45,
                jitter_seconds: 10,
                quiet_hours_utc: Some((1, 6)),
            },
        );
        let p = submit_source_rate(&submit, "greenhouse").unwrap();
        assert_eq!(p.max_per_day, 15);
        assert_eq!(p.min_seconds_between, 45);
        assert_eq!(p.jitter_seconds, 10);
        assert_eq!(p.quiet_hours_utc, Some((1, 6)));
    }
}
