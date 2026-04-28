//! Three-segment prompt builder.
//!
//! Templates live under the workspace root `templates/prompts/`. We
//! render once through a standalone Tera instance (no dependency on
//! `careerai-render`) and split the rendered text into `system` /
//! `profile_block` / `user` along the three marker lines. This split
//! lets `RigLlm` attach Anthropic prompt-cache metadata only to the
//! profile block.

use careerai_core::config::LlmConfig;
use careerai_db::models::Listing;
use careerai_llm::types::LlmRequest;
use careerai_profile::schema::Profile;
use tera::{Context, Tera};

use crate::error::{Result, TailorError};

pub const TAILOR_PROMPT_TEMPLATE: &str =
    include_str!("../../../templates/prompts/tailor_resume.tera");
pub const COVER_LETTER_PROMPT_TEMPLATE: &str =
    include_str!("../../../templates/prompts/cover_letter.tera");

const SYSTEM_MARKER: &str = "<<<SYSTEM>>>";
const PROFILE_MARKER: &str = "<<<PROFILE_BLOCK>>>";
const USER_MARKER: &str = "<<<USER>>>";

const TAILOR_TEMPLATE_NAME: &str = "tailor_resume";
const COVER_LETTER_TEMPLATE_NAME: &str = "cover_letter";

/// Build the tailor-resume `LlmRequest` from fresh inputs. Temperature is
/// held low (0.1) because we want structured JSON, not creative prose.
pub fn tailor_prompt(profile: &Profile, listing: &Listing, cfg: &LlmConfig) -> Result<LlmRequest> {
    let rendered = render(
        TAILOR_TEMPLATE_NAME,
        TAILOR_PROMPT_TEMPLATE,
        profile,
        listing,
        cfg,
    )?;
    let (system, profile_block, user) = split_segments(&rendered)?;
    Ok(LlmRequest {
        system,
        profile_block,
        user,
        prompt_version: cfg.prompt_version.clone(),
        model: cfg.tailor_model.clone(),
        temperature: 0.1,
        max_tokens: 4096,
        cache_profile: cfg.anthropic_prompt_cache,
    })
}

/// Build the cover-letter `LlmRequest`. Same three-segment shape as the
/// tailor prompt; different template + a dedicated `prompt_version`
/// suffix so cache keys don't collide.
pub fn cover_letter_prompt(
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
) -> Result<LlmRequest> {
    let rendered = render(
        COVER_LETTER_TEMPLATE_NAME,
        COVER_LETTER_PROMPT_TEMPLATE,
        profile,
        listing,
        cfg,
    )?;
    let (system, profile_block, user) = split_segments(&rendered)?;
    // Version suffix keeps the cover-letter cache key distinct from the
    // tailor cache key, even though both run off the same `prompt_version`
    // root from config.
    let version = format!("{}+cover_letter", cfg.prompt_version);
    Ok(LlmRequest {
        system,
        profile_block,
        user,
        prompt_version: version,
        model: if cfg.cover_letter_model.is_empty() {
            cfg.tailor_model.clone()
        } else {
            cfg.cover_letter_model.clone()
        },
        temperature: 0.4,
        max_tokens: 1500,
        cache_profile: cfg.anthropic_prompt_cache,
    })
}

fn render(
    name: &str,
    template: &str,
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
) -> Result<String> {
    let profile_yaml = serde_yaml::to_string(profile)?;
    let mut tera = Tera::default();
    tera.add_raw_template(name, template)?;
    let mut ctx = Context::new();
    ctx.insert("prompt_version", &cfg.prompt_version);
    ctx.insert("profile_yaml", &profile_yaml);
    ctx.insert("listing_title", &listing.title);
    ctx.insert("listing_company", &listing.company);
    ctx.insert(
        "listing_location",
        listing.location.as_deref().unwrap_or(""),
    );
    ctx.insert("listing_description", &listing.description);
    let rendered = tera.render(name, &ctx)?;
    Ok(rendered)
}

fn split_segments(rendered: &str) -> Result<(String, String, String)> {
    let sys_idx = rendered.find(SYSTEM_MARKER).ok_or_else(|| {
        TailorError::Profile(format!("template missing segment: {SYSTEM_MARKER}"))
    })?;
    let prof_idx = rendered.find(PROFILE_MARKER).ok_or_else(|| {
        TailorError::Profile(format!("template missing segment: {PROFILE_MARKER}"))
    })?;
    let user_idx = rendered
        .find(USER_MARKER)
        .ok_or_else(|| TailorError::Profile(format!("template missing segment: {USER_MARKER}")))?;
    if !(sys_idx < prof_idx && prof_idx < user_idx) {
        return Err(TailorError::Profile(
            "template segment markers out of order".into(),
        ));
    }
    let system = rendered[sys_idx + SYSTEM_MARKER.len()..prof_idx]
        .trim()
        .to_string();
    let profile_block = rendered[prof_idx + PROFILE_MARKER.len()..user_idx]
        .trim()
        .to_string();
    let user = rendered[user_idx + USER_MARKER.len()..].trim().to_string();
    if system.is_empty() {
        return Err(TailorError::Profile(
            "template empty segment: system".into(),
        ));
    }
    if profile_block.is_empty() {
        return Err(TailorError::Profile(
            "template empty segment: profile_block".into(),
        ));
    }
    if user.is_empty() {
        return Err(TailorError::Profile("template empty segment: user".into()));
    }
    Ok((system, profile_block, user))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_core::config::LlmConfig;
    use careerai_profile::schema::{Experience, Personal, Profile};
    use chrono::Utc;

    fn fixture_cfg() -> LlmConfig {
        LlmConfig {
            tailor_model: "claude-3-5-sonnet".into(),
            cover_letter_model: "claude-3-5-sonnet".into(),
            filter_model: String::new(),
            parse_resume_model: String::new(),
            cache_dir: "data/cache/llm".into(),
            max_retries: 3,
            timeout_seconds: 120,
            prompt_version: "tailor.v1".into(),
            anthropic_prompt_cache: true,
            backend: careerai_core::config::BackendChoice::default(),
        }
    }

    fn fixture_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Ada Lovelace".into(),
                ..Default::default()
            },
            summary: "x".into(),
            skills: careerai_profile::schema::Skills::default(),
            experience: vec![Experience {
                title: "SWE".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2020".into(),
                end: "present".into(),
                bullets: vec!["shipped".into()],
            }],
            education: vec![],
            projects: vec![],
        }
    }

    fn fixture_listing() -> Listing {
        Listing {
            id: "l-1".into(),
            source: "greenhouse".into(),
            external_id: "ext-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com".into(),
            description: "Build things.".into(),
            raw_json: None,
            state: "shortlisted".into(),
            score: Some(0.8),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn tailor_prompt_splits_all_three_segments() {
        let req = tailor_prompt(&fixture_profile(), &fixture_listing(), &fixture_cfg()).unwrap();
        assert!(!req.system.is_empty());
        assert!(!req.profile_block.is_empty());
        assert!(!req.user.is_empty());
        assert!(req.system.contains("exactly one JSON object"));
        assert!(req.profile_block.contains("personal:"));
        assert!(req.user.contains("Senior Rust Engineer"));
        assert_eq!(req.prompt_version, "tailor.v1");
        assert!(req.cache_profile);
    }

    #[test]
    fn cover_letter_prompt_uses_distinct_version() {
        let req =
            cover_letter_prompt(&fixture_profile(), &fixture_listing(), &fixture_cfg()).unwrap();
        assert!(!req.system.is_empty());
        assert!(!req.profile_block.is_empty());
        assert!(!req.user.is_empty());
        assert!(req.prompt_version.ends_with("+cover_letter"));
    }
}
