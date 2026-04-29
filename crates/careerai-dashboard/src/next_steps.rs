use crate::view::{NextStep, NextStepKind, PipelineSnapshot, Urgency};

const SOURCE_STALE_HOURS: u64 = 24;
const COOKIE_WARN_DAYS: u64 = 7;
const PROFILE_STALE_DAYS: u64 = 30;

pub fn compute(snap: &PipelineSnapshot) -> Vec<NextStep> {
    let mut out: Vec<NextStep> = Vec::new();

    if snap.state_counts.shortlisted > 0 {
        let count = snap.state_counts.shortlisted;
        out.push(NextStep {
            kind: NextStepKind::ReadyToTailor { count },
            label: format!("{count} listings ready to tailor"),
            urgency: Urgency::Action,
        });
    }
    if snap.state_counts.tailored > 0 {
        let count = snap.state_counts.tailored;
        out.push(NextStep {
            kind: NextStepKind::ReadyToRender { count },
            label: format!("{count} listings ready to render"),
            urgency: Urgency::Action,
        });
    }
    if snap.state_counts.rendered > 0 {
        let count = snap.state_counts.rendered;
        out.push(NextStep {
            kind: NextStepKind::ReadyToApply { count },
            label: format!("{count} listings ready to apply (dry-run)"),
            urgency: Urgency::Action,
        });
    }

    for (source, hours) in &snap.source_lag_hours {
        if *hours >= SOURCE_STALE_HOURS {
            out.push(NextStep {
                kind: NextStepKind::SourceStale {
                    source: source.clone(),
                    age_hours: *hours,
                },
                // The signal is "no new listings in the DB from this
                // source in N hours", which only loosely correlates
                // with the discover job actually running. A source
                // that runs successfully but yields only duplicates
                // would still trigger this; we phrase the label so an
                // operator reads it that way and looks for the right
                // root cause.
                label: format!("Source `{source}` produced no new listings in {hours}h"),
                urgency: Urgency::Warn,
            });
        }
    }

    if let Some(days) = snap.linkedin_cookie_days_left {
        if days <= COOKIE_WARN_DAYS {
            out.push(NextStep {
                kind: NextStepKind::LinkedInCookieExpiring { days_left: days },
                label: format!("LinkedIn cookie expires in {days}d — refresh"),
                urgency: Urgency::Warn,
            });
        }
    }

    if let Some(age) = snap.profile_age_days {
        if age >= PROFILE_STALE_DAYS {
            out.push(NextStep {
                kind: NextStepKind::ProfileStale { age_days: age },
                label: format!("Profile not refreshed in {age}d"),
                urgency: Urgency::Info,
            });
        }
    }

    out.sort_by(|a, b| {
        a.urgency
            .cmp(&b.urgency)
            .then_with(|| sort_key(&b.kind).cmp(&sort_key(&a.kind)))
    });

    out
}

fn sort_key(kind: &NextStepKind) -> u64 {
    match kind {
        NextStepKind::ReadyToTailor { count }
        | NextStepKind::ReadyToRender { count }
        | NextStepKind::ReadyToApply { count } => *count,
        NextStepKind::SourceStale { age_hours, .. } => *age_hours,
        NextStepKind::LinkedInCookieExpiring { days_left } => u64::MAX - *days_left,
        NextStepKind::ProfileStale { age_days } => *age_days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{KpiStrip, StateCounts};

    fn empty_snapshot() -> PipelineSnapshot {
        PipelineSnapshot {
            kpi: KpiStrip {
                today_discovered: 0,
                shortlisted_active: 0,
                applied_lifetime: 0,
                response_rate_pct: None,
                response_rate_label: "—".to_string(),
            },
            columns: vec![],
            state_counts: StateCounts::default(),
            source_lag_hours: vec![],
            linkedin_cookie_days_left: None,
            profile_age_days: None,
        }
    }

    #[test]
    fn empty_snapshot_yields_no_steps() {
        assert!(compute(&empty_snapshot()).is_empty());
    }

    #[test]
    fn shortlisted_count_yields_ready_to_tailor() {
        let mut s = empty_snapshot();
        s.state_counts.shortlisted = 5;
        let out = compute(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, NextStepKind::ReadyToTailor { count: 5 });
        assert_eq!(out[0].urgency, Urgency::Action);
        assert_eq!(out[0].label, "5 listings ready to tailor");
    }

    #[test]
    fn all_three_ready_states_emit_actions() {
        let mut s = empty_snapshot();
        s.state_counts.shortlisted = 1;
        s.state_counts.tailored = 2;
        s.state_counts.rendered = 3;
        let out = compute(&s);
        assert_eq!(out.len(), 3);
        for step in &out {
            assert_eq!(step.urgency, Urgency::Action);
        }
        assert_eq!(out[0].kind, NextStepKind::ReadyToApply { count: 3 });
        assert_eq!(out[1].kind, NextStepKind::ReadyToRender { count: 2 });
        assert_eq!(out[2].kind, NextStepKind::ReadyToTailor { count: 1 });
    }

    #[test]
    fn source_stale_threshold_24h() {
        let mut s = empty_snapshot();
        s.source_lag_hours = vec![
            ("greenhouse".into(), 23),
            ("lever".into(), 24),
            ("ashby".into(), 48),
        ];
        let out = compute(&s);
        assert_eq!(out.len(), 2);
        for step in &out {
            assert_eq!(step.urgency, Urgency::Warn);
        }
        assert_eq!(
            out[0].kind,
            NextStepKind::SourceStale {
                source: "ashby".into(),
                age_hours: 48
            }
        );
    }

    #[test]
    fn cookie_warning_only_within_seven_days() {
        let mut s = empty_snapshot();
        s.linkedin_cookie_days_left = Some(8);
        assert!(compute(&s).is_empty());
        s.linkedin_cookie_days_left = Some(7);
        assert_eq!(compute(&s).len(), 1);
        s.linkedin_cookie_days_left = Some(0);
        let out = compute(&s);
        assert_eq!(out[0].urgency, Urgency::Warn);
    }

    #[test]
    fn profile_stale_threshold_30d() {
        let mut s = empty_snapshot();
        s.profile_age_days = Some(29);
        assert!(compute(&s).is_empty());
        s.profile_age_days = Some(30);
        let out = compute(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].urgency, Urgency::Info);
    }

    #[test]
    fn ordering_action_before_warn_before_info() {
        let mut s = empty_snapshot();
        s.state_counts.shortlisted = 1;
        s.source_lag_hours = vec![("greenhouse".into(), 48)];
        s.profile_age_days = Some(40);
        let out = compute(&s);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].urgency, Urgency::Action);
        assert_eq!(out[1].urgency, Urgency::Warn);
        assert_eq!(out[2].urgency, Urgency::Info);
    }
}
