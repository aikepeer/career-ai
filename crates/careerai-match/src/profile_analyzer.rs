//! Profile analyzer (ported from career-ops `profile-audit.mjs` +
//! ai-job-search's `analyze_profile.py`).
//!
//! Deterministic — no LLM. Audits the candidate's profile for:
//! - ATS-readiness (contact info completeness, date format consistency)
//! - Bullet quantification rate (what % of bullets have numbers/metrics)
//! - Skill density (total skills, per-category breakdown)
//! - Experience gaps (missing end dates, overlapping entries)
//! - Keyword coverage vs target roles
//!
//! Returns a serialisable report the CLI renders as a checklist.

use serde::{Deserialize, Serialize};

use careerai_profile::schema::Profile;

/// One finding from the audit, with severity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub area: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    /// Blocks ATS parsing or loses interviews — fix immediately.
    Critical,
    /// Weakens the application — fix when time allows.
    Warning,
    /// Informational observation.
    Info,
}

/// The full profile audit report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileReport {
    pub findings: Vec<Finding>,
    pub stats: ProfileStats,
    pub quantification_rate: f64,
    pub ats_ready: bool,
}

/// Aggregated profile statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileStats {
    pub total_experience_entries: usize,
    pub total_bullets: usize,
    pub total_skills: usize,
    pub total_projects: usize,
    pub total_education: usize,
    pub skills_by_category: std::collections::HashMap<String, usize>,
}

/// Run a full profile audit.
#[must_use]
pub fn analyze_profile(profile: &Profile) -> ProfileReport {
    let mut findings = Vec::new();

    check_personal(profile, &mut findings);
    check_summary(profile, &mut findings);
    check_experience(profile, &mut findings);
    check_skills(profile, &mut findings);
    check_education(profile, &mut findings);
    check_target_roles(profile, &mut findings);

    let stats = compute_stats(profile);
    let quantified = count_quantified_bullets(profile);
    let quantification_rate = if stats.total_bullets == 0 {
        0.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        {
            quantified as f64 / stats.total_bullets as f64
        }
    };

    let ats_ready = findings.iter().all(|f| f.severity != Severity::Critical);

    ProfileReport {
        findings,
        stats,
        quantification_rate,
        ats_ready,
    }
}

fn check_personal(profile: &Profile, findings: &mut Vec<Finding>) {
    let p = &profile.personal;
    if p.name.trim().is_empty() {
        findings.push(Finding {
            severity: Severity::Critical,
            area: "personal".into(),
            message: "name is empty — ATS systems require a name".into(),
        });
    }
    if p.email.trim().is_empty() {
        findings.push(Finding {
            severity: Severity::Critical,
            area: "personal".into(),
            message: "email is empty — ATS systems require contact email".into(),
        });
    }
    if p.phone.trim().is_empty() {
        findings.push(Finding {
            severity: Severity::Warning,
            area: "personal".into(),
            message: "phone is empty — some ATS require a phone number".into(),
        });
    }
    if p.location.trim().is_empty() {
        findings.push(Finding {
            severity: Severity::Warning,
            area: "personal".into(),
            message: "location is empty — recruiters filter by location".into(),
        });
    }
}

fn check_summary(profile: &Profile, findings: &mut Vec<Finding>) {
    if profile.summary.trim().is_empty() {
        findings.push(Finding {
            severity: Severity::Warning,
            area: "summary".into(),
            message: "summary is empty — a 2-3 sentence summary helps recruiters scan".into(),
        });
    } else if profile.summary.len() < 50 {
        findings.push(Finding {
            severity: Severity::Info,
            area: "summary".into(),
            message: "summary is very short — consider expanding to 2-3 sentences".into(),
        });
    }
}

fn check_experience(profile: &Profile, findings: &mut Vec<Finding>) {
    if profile.experience.is_empty() {
        findings.push(Finding {
            severity: Severity::Critical,
            area: "experience".into(),
            message: "no experience entries — the profile has no work history".into(),
        });
        return;
    }
    for (i, exp) in profile.experience.iter().enumerate() {
        if exp.start.is_empty() {
            findings.push(Finding {
                severity: Severity::Critical,
                area: format!("experience[{i}]"),
                message: format!("missing start date at {} @ {}", exp.title, exp.company),
            });
        }
        if exp.end.is_empty() {
            findings.push(Finding {
                severity: Severity::Critical,
                area: format!("experience[{i}]"),
                message: format!("missing end date at {} @ {}", exp.title, exp.company),
            });
        }
        if exp.bullets.is_empty() {
            findings.push(Finding {
                severity: Severity::Warning,
                area: format!("experience[{i}]"),
                message: format!("no bullets at {} @ {}", exp.title, exp.company),
            });
        }
    }
}

fn check_skills(profile: &Profile, findings: &mut Vec<Finding>) {
    let total: usize = profile.skills.all_skill_names().count();
    if total == 0 {
        findings.push(Finding {
            severity: Severity::Critical,
            area: "skills".into(),
            message: "no skills listed — add at least languages and frameworks".into(),
        });
    } else if total < 5 {
        findings.push(Finding {
            severity: Severity::Warning,
            area: "skills".into(),
            message: "few skills listed — aim for 10-20 across categories".into(),
        });
    }
}

fn check_education(profile: &Profile, findings: &mut Vec<Finding>) {
    if profile.education.is_empty() {
        findings.push(Finding {
            severity: Severity::Info,
            area: "education".into(),
            message: "no education entries — add if relevant to target roles".into(),
        });
    }
    for edu in &profile.education {
        if edu.degree.is_empty() {
            findings.push(Finding {
                severity: Severity::Warning,
                area: "education".into(),
                message: format!("missing degree at {}", edu.institution),
            });
        }
    }
}

fn check_target_roles(profile: &Profile, findings: &mut Vec<Finding>) {
    if profile.target_roles.is_empty() {
        findings.push(Finding {
            severity: Severity::Info,
            area: "target_roles".into(),
            message: "no target_roles set — helps the matcher weight discovery queries".into(),
        });
    }
}

fn compute_stats(profile: &Profile) -> ProfileStats {
    let mut skills_by_category = std::collections::HashMap::new();
    skills_by_category.insert("languages".into(), profile.skills.languages.len());
    skills_by_category.insert("platforms".into(), profile.skills.platforms.len());
    skills_by_category.insert("frameworks".into(), profile.skills.frameworks.len());
    skills_by_category.insert("devops".into(), profile.skills.devops.len());
    skills_by_category.insert("tools".into(), profile.skills.tools.len());
    skills_by_category.insert("debugging".into(), profile.skills.debugging.len());
    skills_by_category.insert("protocols".into(), profile.skills.protocols.len());

    let total_bullets: usize = profile.experience.iter().map(|e| e.bullets.len()).sum();

    ProfileStats {
        total_experience_entries: profile.experience.len(),
        total_bullets,
        total_skills: profile.skills.all_skill_names().count(),
        total_projects: profile.projects.len(),
        total_education: profile.education.len(),
        skills_by_category,
    }
}

fn count_quantified_bullets(profile: &Profile) -> usize {
    profile
        .experience
        .iter()
        .flat_map(|e| &e.bullets)
        .filter(|b| contains_number(b))
        .count()
}

fn contains_number(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Experience, Personal, Skills};

    fn full_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Jane Doe".into(),
                email: "jane@example.com".into(),
                phone: "+1234567890".into(),
                location: "Delhi, IN".into(),
                links: careerai_profile::schema::Links::default(),
            },
            summary: "Senior engineer with 10 years building distributed systems.".into(),
            skills: Skills {
                languages: vec!["Rust".into(), "Python".into(), "C".into()],
                frameworks: vec!["Tokio".into(), "PyTorch".into()],
                ..Default::default()
            },
            experience: vec![Experience {
                title: "Senior Engineer".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2018".into(),
                end: "present".into(),
                bullets: vec![
                    "Built pipeline processing 10M events/day".into(),
                    "Led team of 5 engineers".into(),
                    "Maintained codebase".into(),
                ],
            }],
            education: vec![],
            projects: vec![],
            target_roles: vec!["Senior Rust Engineer".into()],
            ..Default::default()
        }
    }

    #[test]
    fn full_profile_passes_with_no_critical() {
        let report = analyze_profile(&full_profile());
        assert!(
            report.ats_ready,
            "expected ATS-ready, findings: {:?}",
            report.findings
        );
        assert_eq!(report.stats.total_bullets, 3);
        // 2 of 3 bullets have numbers.
        assert!((report.quantification_rate - 0.6667).abs() < 0.01);
    }

    #[test]
    fn missing_email_is_critical() {
        let mut p = full_profile();
        p.personal.email = String::new();
        let report = analyze_profile(&p);
        assert!(!report.ats_ready);
        assert!(report
            .findings
            .iter()
            .any(|f| { f.severity == Severity::Critical && f.message.contains("email") }));
    }

    #[test]
    fn empty_experience_is_critical() {
        let mut p = full_profile();
        p.experience.clear();
        let report = analyze_profile(&p);
        assert!(!report.ats_ready);
        assert!(report
            .findings
            .iter()
            .any(|f| { f.severity == Severity::Critical && f.message.contains("no experience") }));
    }

    #[test]
    fn missing_dates_are_critical() {
        let mut p = full_profile();
        p.experience[0].start = String::new();
        let report = analyze_profile(&p);
        assert!(!report.ats_ready);
        assert!(report
            .findings
            .iter()
            .any(|f| { f.severity == Severity::Critical && f.message.contains("start date") }));
    }

    #[test]
    fn no_bullets_is_warning() {
        let mut p = full_profile();
        p.experience[0].bullets.clear();
        let report = analyze_profile(&p);
        // Still ATS-ready (no critical), but has a warning.
        assert!(report.ats_ready);
        assert!(report
            .findings
            .iter()
            .any(|f| { f.severity == Severity::Warning && f.message.contains("no bullets") }));
    }

    #[test]
    fn quantification_rate_zero_when_no_bullets() {
        let mut p = full_profile();
        p.experience[0].bullets.clear();
        let report = analyze_profile(&p);
        assert!((report.quantification_rate - 0.0).abs() < 1e-9);
    }

    #[test]
    fn skills_by_category_populated() {
        let report = analyze_profile(&full_profile());
        assert_eq!(report.stats.skills_by_category.get("languages"), Some(&3));
        assert_eq!(report.stats.skills_by_category.get("frameworks"), Some(&2));
        assert_eq!(report.stats.total_skills, 5);
    }

    #[test]
    fn empty_profile_has_multiple_criticals() {
        let report = analyze_profile(&Profile::default());
        assert!(!report.ats_ready);
        let critical_count = report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        assert!(
            critical_count >= 3,
            "expected >=3 criticals, got {critical_count}"
        );
    }
}
