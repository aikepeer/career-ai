//! `careerai analyze-profile` — audit the candidate profile for ATS
//! readiness, quantification rate, and completeness.

use anyhow::Result;
use careerai_pipeline::load_profile;

use crate::load_cfg;

/// Run the profile audit and print a checklist.
pub fn run_analyze_profile(cwd: &std::path::Path) -> Result<()> {
    let _cfg = load_cfg(cwd)?;
    let profile = load_profile(cwd)?;
    let report = careerai_match::analyze_profile(&profile);

    println!("=== Profile Audit ===\n");
    println!("ATS Ready: {}", if report.ats_ready { "YES" } else { "NO" });
    println!(
        "Quantification Rate: {:.0}% ({} of {} bullets have metrics)",
        report.quantification_rate * 100.0,
        count_quantified(&profile),
        report.stats.total_bullets,
    );
    println!();

    println!("--- Stats ---\n");
    println!(
        "  Experience entries: {}",
        report.stats.total_experience_entries
    );
    println!("  Total bullets:       {}", report.stats.total_bullets);
    println!("  Total skills:        {}", report.stats.total_skills);
    println!("  Projects:            {}", report.stats.total_projects);
    println!("  Education entries:   {}", report.stats.total_education);
    println!();

    println!("--- Skills by Category ---\n");
    let mut cats: Vec<_> = report.stats.skills_by_category.iter().collect();
    cats.sort_by_key(|(k, _)| k.as_str());
    for (cat, count) in &cats {
        println!("  {cat:<15} {count}");
    }
    println!();

    println!("--- Findings ({}) ---\n", report.findings.len());
    for f in &report.findings {
        let icon = match f.severity {
            careerai_match::Severity::Critical => "[CRIT]",
            careerai_match::Severity::Warning => "[WARN]",
            careerai_match::Severity::Info => "[INFO]",
        };
        println!(
            "  {icon} {area}: {message}",
            area = f.area,
            message = f.message
        );
    }

    if report.ats_ready {
        println!("\nProfile passes ATS readiness checks.");
    } else {
        let criticals = report
            .findings
            .iter()
            .filter(|f| f.severity == careerai_match::Severity::Critical)
            .count();
        println!("\n{criticals} critical issue(s) must be fixed before applying.");
    }

    Ok(())
}

fn count_quantified(profile: &careerai_profile::schema::Profile) -> usize {
    profile
        .experience
        .iter()
        .flat_map(|e| &e.bullets)
        .filter(|b| b.chars().any(|c| c.is_ascii_digit()))
        .count()
}
