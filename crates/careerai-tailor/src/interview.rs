//! Interview preparation (ported from career-ops interview suite + ai-job-search
//! `/interview`).
//!
//! Given an application's listing + tailored resume + cover letter, builds:
//! - a STAR story bank from the candidate's profile
//! - likely interview questions derived from the JD
//! - a mapping of questions → best STAR stories
//!
//! The LLM call is advisory; the STAR stories are extracted from the
//! profile and surfaced to the LLM as context. No invented experience.

use serde::{Deserialize, Serialize};

use careerai_core::config::LlmConfig;
use careerai_db::models::Listing;
use careerai_llm::trait_def::Llm;
use careerai_llm::types::{LlmRequest, LlmResponse};
use careerai_profile::schema::Profile;

use crate::error::{Result, TailorError};

/// One STAR story extracted from the candidate's profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarStory {
    pub situation: String,
    pub task: String,
    pub action: String,
    pub result: String,
    /// Which experience entry this came from.
    pub source: String,
}

/// A likely interview question mapped to the best-matching STAR story.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewQuestion {
    pub question: String,
    /// Index into the `story_bank` vec.
    #[serde(default)]
    pub best_story_idx: Option<usize>,
    /// Brief prep note for this question.
    #[serde(default)]
    pub prep_note: Option<String>,
}

/// The full interview prep pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewPrep {
    pub star_stories: Vec<StarStory>,
    pub questions: Vec<InterviewQuestion>,
    pub general_advice: String,
}

/// Extract STAR stories from the profile's experience bullets. Each bullet
/// becomes a candidate STAR story with the action filled from the bullet
/// text. This is deterministic — no LLM call.
#[must_use]
pub fn extract_star_stories(profile: &Profile) -> Vec<StarStory> {
    let mut stories = Vec::new();
    for exp in &profile.experience {
        for bullet in &exp.bullets {
            stories.push(StarStory {
                situation: format!("At {} as {}", exp.company, exp.title),
                task: bullet.clone(),
                action: bullet.clone(),
                result: String::new(),
                source: format!("{} @ {}", exp.title, exp.company),
            });
        }
    }
    stories
}

/// Build the interview-prep LLM request.
pub fn interview_prompt(
    profile: &Profile,
    listing: &Listing,
    stories: &[StarStory],
    cfg: &LlmConfig,
) -> Result<LlmRequest> {
    let stories_json = serde_json::to_string_pretty(stories)
        .map_err(|e| TailorError::Schema(format!("serialize stories: {e}")))?;

    let system = "You are an interview coach. Given the candidate's STAR story bank and a job description, generate 8-10 likely interview questions and map each to the best STAR story. Respond ONLY as valid JSON matching the InterviewPrep schema. Never invent experience — only use the provided stories.";

    let user = format!(
        "## Job Description\nCompany: {company}\nTitle: {title}\n\n{jd}\n\n\
         ## Candidate STAR Stories\n{stories_json}\n\n\
         ## Candidate Summary\n{summary}\n\n\
         Generate the interview prep pack as JSON:\n\
         {{\n  \
         \"star_stories\": [...the provided stories...],\n  \
         \"questions\": [{{\"question\": \"...\", \"best_story_idx\": 0, \"prep_note\": \"...\"}}],\n  \
         \"general_advice\": \"...\"\n\
         }}",
        company = listing.company,
        title = listing.title,
        jd = listing.description,
        stories_json = stories_json,
        summary = profile.summary,
    );

    Ok(LlmRequest {
        system: system.to_string(),
        profile_block: String::new(),
        user,
        prompt_version: format!("{}+interview", cfg.prompt_version),
        model: cfg.tailor_model.clone(),
        temperature: 0.3,
        max_tokens: 8192,
        cache_profile: false,
    })
}

/// Run the interview-prep LLM call. Returns the parsed prep pack.
pub async fn generate_interview_prep(
    llm: &(dyn Llm + Send + Sync),
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
) -> Result<InterviewPrep> {
    let stories = extract_star_stories(profile);
    let req = interview_prompt(profile, listing, &stories, cfg)?;
    let resp: LlmResponse = llm.complete(&req).await?;
    parse_prep(&resp.text, stories)
}

fn parse_prep(raw: &str, fallback_stories: Vec<StarStory>) -> Result<InterviewPrep> {
    let trimmed = raw.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    let mut prep: InterviewPrep = serde_json::from_str(json_str)
        .map_err(|e| TailorError::Schema(format!("interview prep parse: {e}")))?;
    // If the LLM didn't return stories, use the deterministic extraction.
    if prep.star_stories.is_empty() {
        prep.star_stories = fallback_stories;
    }
    Ok(prep)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn extract_stories_from_experience() {
        let mut p = Profile::default();
        p.experience.push(careerai_profile::schema::Experience {
            title: "Engineer".into(),
            company: "Acme".into(),
            location: String::new(),
            start: "2020".into(),
            end: "present".into(),
            bullets: vec!["Built a pipeline".into(), "Shipped 10M events/day".into()],
        });
        let stories = extract_star_stories(&p);
        assert_eq!(stories.len(), 2);
        assert!(stories[0].action.contains("pipeline"));
        assert_eq!(stories[0].source, "Engineer @ Acme");
    }

    #[test]
    fn parse_valid_prep() {
        let raw = r#"```json
        {
            "star_stories": [],
            "questions": [
                {"question": "Tell me about a challenge", "best_story_idx": 0, "prep_note": "Focus on the technical depth"}
            ],
            "general_advice": "Practice out loud"
        }
        ```"#;
        let stories = vec![StarStory {
            situation: "s".into(),
            task: "t".into(),
            action: "a".into(),
            result: "r".into(),
            source: "src".into(),
        }];
        let prep = parse_prep(raw, stories).unwrap();
        assert_eq!(prep.questions.len(), 1);
        assert_eq!(prep.general_advice, "Practice out loud");
        // Fallback stories used because LLM returned empty.
        assert_eq!(prep.star_stories.len(), 1);
    }
}
