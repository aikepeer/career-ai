//! Cover letter tone matching.
//!
//! Detects company-culture signals from the company name and job
//! description to select the most appropriate cover letter tone.

#![allow(clippy::cast_precision_loss, clippy::needless_pass_by_value)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterTone {
    /// Large enterprise, traditional.
    Formal,
    /// Startup, small company.
    Casual,
    /// Non-profit, sustainability, healthcare.
    MissionDriven,
    /// Engineering-heavy, deep tech.
    Technical,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToneAnalysis {
    pub tone: CoverLetterTone,
    pub confidence: f32,
    pub signals: Vec<String>,
}

const FORMAL_KEYWORDS: &[&str] = &[
    "enterprise",
    "corporation",
    "fortune",
    "bank",
    "insurance",
    "government",
    "compliance",
    "regulated",
    "established",
    "global leader",
];

const CASUAL_KEYWORDS: &[&str] = &[
    "startup",
    "small team",
    "flat hierarchy",
    "remote-first",
    "fast-paced",
    "wear many hats",
    "agile",
    "founded in 20",
];

const MISSION_KEYWORDS: &[&str] = &[
    "mission",
    "impact",
    "sustainability",
    "non-profit",
    "healthcare",
    "education",
    "climate",
    "social",
    "community",
    "make a difference",
];

const TECHNICAL_KEYWORDS: &[&str] = &[
    "engineering",
    "infrastructure",
    "platform",
    "deep tech",
    "systems",
    "low-level",
    "compiler",
    "runtime",
    "distributed",
    "scale",
    "performance",
    "embedded",
    "robotics",
];

/// Count keyword matches in the combined text and return matched keywords.
fn count_matches(text: &str, keywords: &[&'static str]) -> (u32, Vec<String>) {
    let mut signals = Vec::new();
    let mut count = 0;
    for kw in keywords {
        if text.contains(kw) {
            count += 1;
            signals.push((*kw).to_string());
        }
    }
    (count, signals)
}

/// Detect the best cover letter tone for a company + description.
pub fn detect_tone(company: &str, description: &str) -> ToneAnalysis {
    let combined = format!("{company} {description}").to_lowercase();

    let (formal_count, formal_signals) = count_matches(&combined, FORMAL_KEYWORDS);
    let (casual_count, casual_signals) = count_matches(&combined, CASUAL_KEYWORDS);
    let (mission_count, mission_signals) = count_matches(&combined, MISSION_KEYWORDS);
    let (tech_count, tech_signals) = count_matches(&combined, TECHNICAL_KEYWORDS);

    let total = formal_count + casual_count + mission_count + tech_count;

    // Pick the highest score; ties prefer Formal.
    let candidates: [(CoverLetterTone, u32, &Vec<String>); 4] = [
        (CoverLetterTone::Formal, formal_count, &formal_signals),
        (CoverLetterTone::Casual, casual_count, &casual_signals),
        (
            CoverLetterTone::MissionDriven,
            mission_count,
            &mission_signals,
        ),
        (CoverLetterTone::Technical, tech_count, &tech_signals),
    ];

    let empty_signals = Vec::new();
    let (tone, best_count, signals) = candidates
        .into_iter()
        .reduce(|acc, cur| if cur.1 > acc.1 { cur } else { acc })
        .unwrap_or((CoverLetterTone::Formal, 0, &empty_signals));

    let confidence = best_count as f32 / (total as f32 + 1.0);

    ToneAnalysis {
        tone,
        confidence,
        signals: signals.clone(),
    }
}

/// Convert a tone into a content-library query suffix.
pub fn tone_label(tone: CoverLetterTone) -> &'static str {
    match tone {
        CoverLetterTone::Formal => "formal",
        CoverLetterTone::Casual => "casual",
        CoverLetterTone::MissionDriven => "mission",
        CoverLetterTone::Technical => "technical",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_startup_casual() {
        let analysis = detect_tone(
            "QuickStart",
            "We are a fast-paced startup with a small team. Remote-first, \
             flat hierarchy where everyone wears many hats. Founded in 2019.",
        );
        assert_eq!(analysis.tone, CoverLetterTone::Casual);
        assert!(!analysis.signals.is_empty());
    }

    #[test]
    fn test_detects_enterprise_formal() {
        let analysis = detect_tone(
            "Global Bank Corporation",
            "A global leader in regulated financial services and insurance. \
             Fortune 500 enterprise with strict compliance requirements.",
        );
        assert_eq!(analysis.tone, CoverLetterTone::Formal);
        assert!(!analysis.signals.is_empty());
    }

    #[test]
    fn test_detects_mission_driven() {
        let analysis = detect_tone(
            "GreenFuture",
            "Our mission is to drive sustainability and climate action. \
             A non-profit making a difference in healthcare and education \
             through community impact.",
        );
        assert_eq!(analysis.tone, CoverLetterTone::MissionDriven);
        assert!(!analysis.signals.is_empty());
    }

    #[test]
    fn test_detects_technical() {
        let analysis = detect_tone(
            "DeepTech Systems",
            "We build engineering infrastructure and distributed systems \
             at scale. Low-level compiler and runtime performance work, \
             embedded robotics platform.",
        );
        assert_eq!(analysis.tone, CoverLetterTone::Technical);
        assert!(!analysis.signals.is_empty());
    }

    #[test]
    fn test_default_formal_on_tie() {
        // "agile" matches Casual, "government" matches Formal — each tone
        // scores exactly 1, so Formal should win the tie.
        let analysis = detect_tone("Corp", "We are agile. Government contracts.");
        assert_eq!(analysis.tone, CoverLetterTone::Formal);
    }
}
