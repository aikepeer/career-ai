//! Salary negotiation script generator (ported from career-ops
//! `negotiation-coach.mjs` + job_agentic's `negotiation/` module).
//!
//! Given a listing + profile + optional salary benchmark, generates:
//! - a counter-offer email script
//! - negotiation talking points (anchored on market data + leverage)
//! - a "walk-away" threshold reminder
//!
//! LLM-based. Safety invariant: the script never invents salary numbers —
//! it uses the profile's `compensation` block or the salary benchmark as
//! the anchor, and only suggests a percentage range above that anchor.

use serde::{Deserialize, Serialize};

use careerai_core::config::LlmConfig;
use careerai_db::models::Listing;
use careerai_llm::trait_def::Llm;
use careerai_llm::types::{LlmRequest, LlmResponse};
use careerai_profile::schema::Profile;

use crate::error::{Result, TailorError};

/// One talking point for the negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalkingPoint {
    pub topic: String,
    pub script: String,
}

/// The full negotiation script pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationScript {
    /// The counter-offer email body (no subject — caller adds it).
    pub counter_offer_email: String,
    /// Key talking points for a live conversation.
    pub talking_points: Vec<TalkingPoint>,
    /// The minimum acceptable offer (walk-away threshold).
    pub walkaway_threshold: String,
    /// Suggested counter range as a percentage above the initial offer.
    /// E.g. `10..=15` means "counter 10-15% above the initial offer".
    pub counter_percent_low: u8,
    pub counter_percent_high: u8,
}

/// Parse a compensation target from the profile's `compensation` block.
/// Returns `(low, high)` as f64 if parseable.
fn profile_comp_range(profile: &Profile) -> Option<(f64, f64)> {
    let comp = &profile.compensation;
    if !comp.target_range.is_empty() {
        return parse_range(&comp.target_range);
    }
    None
}

fn parse_range(range: &str) -> Option<(f64, f64)> {
    let digits: Vec<f64> = range
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|tok| {
            let tok = tok.trim();
            if tok.is_empty() {
                return None;
            }
            let (num_part, mult) = match tok.chars().last() {
                Some('k' | 'K') => (&tok[..tok.len() - 1], 1000.0),
                _ => (tok, 1.0),
            };
            num_part.parse::<f64>().ok().map(|n| n * mult)
        })
        .collect();
    match digits.len() {
        2 => Some((digits[0], digits[1])),
        1 => Some((digits[0], digits[0])),
        _ => None,
    }
}

/// Build the negotiation-script LLM request.
pub fn negotiation_prompt(
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
    benchmark: Option<&str>,
) -> LlmRequest {
    let comp_info = match profile_comp_range(profile) {
        Some((low, high)) => {
            format!("Candidate target: ${low:.0} - ${high:.0}")
        }
        None => "No explicit target in profile.".to_string(),
    };

    let benchmark_info = if let Some(b) = benchmark {
        format!("Market benchmark: {b}")
    } else {
        "No market benchmark available.".to_string()
    };

    let system = "You are a salary negotiation coach. Generate a counter-offer email and talking points. Be professional, evidence-based, and never aggressive. NEVER invent salary numbers — use only the provided target range and benchmark. Respond ONLY as valid JSON matching the NegotiationScript schema.";

    let user = format!(
        "## Job\nCompany: {company}\nTitle: {title}\nLocation: {location}\n\n\
         ## Compensation Context\n{comp_info}\n{benchmark_info}\n\n\
         ## Candidate Leverage\n\
         Name: {name}\n\
         Summary: {summary}\n\
         Years of experience: {years}\n\
         Key skills: {skills}\n\n\
         Generate a negotiation script pack as JSON:\n\
         {{\n  \
         \"counter_offer_email\": \"...\",\n  \
         \"talking_points\": [{{\"topic\": \"...\", \"script\": \"...\"}}],\n  \
         \"walkaway_threshold\": \"...\",\n  \
         \"counter_percent_low\": 10,\n  \
         \"counter_percent_high\": 15\n\
         }}\n\n\
         Rules:\n\
         - Counter 10-20% above the initial offer, never more.\n\
         - The walkaway threshold must be at or below the candidate's target_low.\n\
         - Talking points must reference the candidate's actual experience.\n\
         - No buzzwords ('excited', 'thrilled', 'passionate').",
        company = listing.company,
        title = listing.title,
        location = listing.location.as_deref().unwrap_or("Not specified"),
        comp_info = comp_info,
        benchmark_info = benchmark_info,
        name = profile.personal.name,
        summary = profile.summary,
        years = profile.experience.len(),
        skills = profile
            .skills
            .all_skill_names()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    );

    LlmRequest {
        system: system.to_string(),
        profile_block: String::new(),
        user,
        prompt_version: format!("{}+negotiation", cfg.prompt_version),
        model: cfg.tailor_model.clone(),
        temperature: 0.3,
        max_tokens: 4096,
        cache_profile: false,
    }
}

/// Generate the negotiation script via the LLM.
pub async fn generate_negotiation_script(
    llm: &(dyn Llm + Send + Sync),
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
    benchmark: Option<&str>,
) -> Result<NegotiationScript> {
    let req = negotiation_prompt(profile, listing, cfg, benchmark);
    let resp: LlmResponse = llm.complete(&req).await?;
    parse_script(&resp.text)
}

fn parse_script(raw: &str) -> Result<NegotiationScript> {
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
    let script: NegotiationScript = serde_json::from_str(json_str)
        .map_err(|e| TailorError::Schema(format!("negotiation parse: {e}")))?;
    Ok(script)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_script() {
        let raw = r#"```json
        {
            "counter_offer_email": "Dear Acme,\n\nThank you for the offer...",
            "talking_points": [
                {"topic": "Market rate", "script": "Based on my research..."}
            ],
            "walkaway_threshold": "Do not go below $140K",
            "counter_percent_low": 12,
            "counter_percent_high": 18
        }
        ```"#;
        let script = parse_script(raw).unwrap();
        assert!(script.counter_offer_email.contains("Thank you"));
        assert_eq!(script.talking_points.len(), 1);
        assert_eq!(script.counter_percent_low, 12);
        assert_eq!(script.counter_percent_high, 18);
    }

    #[test]
    fn parse_without_fences() {
        let raw = r#"{"counter_offer_email": "x", "talking_points": [], "walkaway_threshold": "y", "counter_percent_low": 10, "counter_percent_high": 15}"#;
        let script = parse_script(raw).unwrap();
        assert_eq!(script.counter_offer_email, "x");
    }

    #[test]
    fn parse_rejects_invalid() {
        assert!(parse_script("not json").is_err());
    }

    #[test]
    fn parse_range_dollar_k() {
        assert_eq!(parse_range("$150K-200K"), Some((150_000.0, 200_000.0)));
    }

    #[test]
    fn parse_range_plain() {
        assert_eq!(parse_range("150000-200000"), Some((150_000.0, 200_000.0)));
    }

    #[test]
    fn parse_range_single() {
        assert_eq!(parse_range("150K"), Some((150_000.0, 150_000.0)));
    }

    #[test]
    fn parse_range_invalid() {
        assert_eq!(parse_range("negotiable"), None);
    }
}
