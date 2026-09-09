//! Skill gap heatmap — cross-reference the user's skills against job
//! descriptions in shortlisted listings to find which skills appear most
//! often in JDs but are missing from the profile.

#![allow(clippy::cast_precision_loss, clippy::too_many_lines)]

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillGapReport {
    pub missing_skills: Vec<SkillGapEntry>,
    pub total_jds_analyzed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillGapEntry {
    pub skill: String,
    pub frequency: usize,
    pub percentage: f32,
    pub category: String,
}

/// Known tech skills grouped by category. Used to scan JD text for
/// skill mentions.
struct SkillDb {
    entries: Vec<(&'static str, &'static str)>, // (skill, category)
}

impl SkillDb {
    fn new() -> Self {
        let entries: [(&str, &str); 98] = [
            // Languages
            ("rust", "language"),
            ("python", "language"),
            ("go", "language"),
            ("c++", "language"),
            ("java", "language"),
            ("javascript", "language"),
            ("typescript", "language"),
            ("kotlin", "language"),
            ("swift", "language"),
            ("ruby", "language"),
            ("scala", "language"),
            ("c#", "language"),
            ("c", "language"),
            ("elixir", "language"),
            ("haskell", "language"),
            ("clojure", "language"),
            ("perl", "language"),
            ("php", "language"),
            ("dart", "language"),
            ("lua", "language"),
            ("julia", "language"),
            // Frameworks
            ("tokio", "framework"),
            ("actix", "framework"),
            ("react", "framework"),
            ("vue", "framework"),
            ("angular", "framework"),
            ("django", "framework"),
            ("flask", "framework"),
            ("spring", "framework"),
            ("express", "framework"),
            ("nestjs", "framework"),
            ("fastapi", "framework"),
            ("svelte", "framework"),
            ("next.js", "framework"),
            ("nuxt", "framework"),
            ("rocket", "framework"),
            ("axum", "framework"),
            ("gin", "framework"),
            ("rails", "framework"),
            // Tools / DevOps
            ("docker", "tool"),
            ("kubernetes", "tool"),
            ("k8s", "tool"),
            ("terraform", "tool"),
            ("ansible", "tool"),
            ("jenkins", "tool"),
            ("gitlab ci", "tool"),
            ("github actions", "tool"),
            ("prometheus", "tool"),
            ("grafana", "tool"),
            ("elasticsearch", "tool"),
            ("redis", "tool"),
            ("kafka", "tool"),
            ("rabbitmq", "tool"),
            ("nginx", "tool"),
            ("haproxy", "tool"),
            ("vault", "tool"),
            ("consul", "tool"),
            ("circleci", "tool"),
            // Cloud platforms
            ("aws", "platform"),
            ("gcp", "platform"),
            ("azure", "platform"),
            ("digitalocean", "platform"),
            ("linode", "platform"),
            ("heroku", "platform"),
            ("vercel", "platform"),
            ("cloudflare", "platform"),
            // OS / embedded
            ("linux", "platform"),
            ("android", "platform"),
            ("ios", "platform"),
            ("embedded", "platform"),
            ("rtos", "platform"),
            ("stm32", "platform"),
            ("raspberry pi", "platform"),
            ("arduino", "platform"),
            ("fpga", "platform"),
            // Concepts
            ("machine learning", "concept"),
            ("deep learning", "concept"),
            ("llm", "concept"),
            ("nlp", "concept"),
            ("computer vision", "concept"),
            ("distributed systems", "concept"),
            ("microservices", "concept"),
            ("event-driven", "concept"),
            ("system design", "concept"),
            ("data structures", "concept"),
            ("algorithms", "concept"),
            ("concurrency", "concept"),
            ("async", "concept"),
            ("compilers", "concept"),
            ("operating systems", "concept"),
            ("networking", "concept"),
            ("security", "concept"),
            ("cryptography", "concept"),
            ("blockchain", "concept"),
            ("data engineering", "concept"),
            ("etl", "concept"),
            ("reinforcement learning", "concept"),
            ("mlops", "concept"),
        ];
        Self {
            entries: entries.to_vec(),
        }
    }
}

/// Analyze skill gaps between JDs and the user's skills.
pub fn analyze_skill_gaps(jd_texts: &[String], user_skills: &[String]) -> SkillGapReport {
    let db = SkillDb::new();
    let user_lower: Vec<String> = user_skills.iter().map(|s| s.to_ascii_lowercase()).collect();
    let total = jd_texts.len();

    let mut freq: std::collections::HashMap<&str, (usize, &str)> = std::collections::HashMap::new();

    for jd in jd_texts {
        let jd_lower = jd.to_ascii_lowercase();
        for (skill, category) in &db.entries {
            if jd_lower.contains(skill) {
                freq.entry(skill).or_insert((0, *category)).0 += 1;
            }
        }
    }

    let mut missing: Vec<SkillGapEntry> = freq
        .into_iter()
        .filter(|(skill, _)| {
            !user_lower
                .iter()
                .any(|u| u.contains(*skill) || skill.contains(u.as_str()))
        })
        .map(|(skill, (count, category))| SkillGapEntry {
            skill: skill.to_string(),
            frequency: count,
            percentage: if total > 0 {
                count as f32 / total as f32 * 100.0
            } else {
                0.0
            },
            category: category.to_string(),
        })
        .collect();

    missing.sort_by_key(|a| std::cmp::Reverse(a.frequency));
    missing.truncate(20);

    SkillGapReport {
        missing_skills: missing,
        total_jds_analyzed: total,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_missing_rust() {
        let jds = vec!["We need a Rust engineer with Tokio experience.".to_string()];
        let skills = vec!["Python".to_string()];
        let report = analyze_skill_gaps(&jds, &skills);
        let rust = report.missing_skills.iter().find(|e| e.skill == "rust");
        assert!(rust.is_some(), "Rust should be in missing skills");
        assert_eq!(rust.unwrap().frequency, 1);
    }

    #[test]
    fn test_excludes_existing_skills() {
        let jds = vec!["We need a Rust engineer with Python experience.".to_string()];
        let skills = vec!["Rust".to_string(), "Python".to_string()];
        let report = analyze_skill_gaps(&jds, &skills);
        assert!(!report.missing_skills.iter().any(|e| e.skill == "rust"));
        assert!(!report.missing_skills.iter().any(|e| e.skill == "python"));
    }

    #[test]
    fn test_empty_jds() {
        let report = analyze_skill_gaps(&[], &["Rust".to_string()]);
        assert!(report.missing_skills.is_empty());
        assert_eq!(report.total_jds_analyzed, 0);
    }

    #[test]
    fn test_percentage_calculated() {
        let jds = vec![
            "Need Rust and Docker".to_string(),
            "Need Rust and Python".to_string(),
            "Need Go".to_string(),
        ];
        let report = analyze_skill_gaps(&jds, &[]);
        let rust = report
            .missing_skills
            .iter()
            .find(|e| e.skill == "rust")
            .unwrap();
        assert!((rust.percentage - 66.67).abs() < 0.1 || (rust.percentage - 66.0).abs() < 1.0);
    }
}
