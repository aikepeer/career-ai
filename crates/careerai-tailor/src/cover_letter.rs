//! Cover-letter drafter.
//!
//! Reuses the tailor cache so a rerun on the same (profile, listing,
//! model, prompt_version) is free. Enforces a hard 350-whitespace-word
//! cap — over that we reject rather than auto-truncate so users notice
//! a runaway response and can adjust the prompt.

use careerai_core::config::LlmConfig;
use careerai_db::models::Listing;
use careerai_llm::cache::Cache;
use careerai_llm::hashing::{compose_key, jd_hash};
use careerai_llm::trait_def::Llm;
use careerai_profile::schema::Profile;
use tracing::info;

use crate::error::{Result, TailorError};
use crate::model::CoverLetter;
use crate::prompt::cover_letter_prompt;

pub const COVER_LETTER_WORD_CAP: usize = 350;
/// Char cap companion to the word cap — catches LLMs emitting one giant
/// no-whitespace blob that would pass the word count check.
pub const COVER_LETTER_CHAR_CAP: usize = 3500;

/// Draft a cover letter via the LLM (cache first). Errors:
///
/// - `CoverLetterTooLong` if > 350 whitespace-split words OR > 3500 chars.
/// - `Llm` on provider / upstream failure.
///
/// `profile_hash` is provided by the caller (already computed in
/// `tailor_for_listing`) to avoid recomputing the canonical-JSON hash.
pub async fn draft(
    llm: &(dyn Llm + Send + Sync),
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
    cache: &Cache,
    profile_hash: &str,
) -> Result<CoverLetter> {
    let req = cover_letter_prompt(profile, listing, cfg)?;
    let jd = jd_hash(&listing.title, &listing.company, &listing.description);
    let key = compose_key(&req.prompt_version, profile_hash, &jd, &req.model);

    let resp = if let Some(hit) = cache.get(&key).await? {
        info!(
            target: "tailor",
            cache = "hit",
            prompt_version = %req.prompt_version,
            "cover_letter cache hit"
        );
        hit
    } else {
        info!(
            target: "tailor",
            cache = "miss",
            prompt_version = %req.prompt_version,
            "cover_letter calling llm"
        );
        let fresh = llm.complete(&req).await?;
        cache.put(&key, &fresh).await?;
        fresh
    };

    let body = resp.text.trim().to_string();
    let chars = body.chars().count();
    if chars > COVER_LETTER_CHAR_CAP {
        return Err(TailorError::CoverLetterCharsTooLong {
            chars,
            cap: COVER_LETTER_CHAR_CAP,
        });
    }
    let words = body.split_whitespace().count();
    if words > COVER_LETTER_WORD_CAP {
        return Err(TailorError::CoverLetterTooLong {
            words,
            cap: COVER_LETTER_WORD_CAP,
        });
    }
    Ok(CoverLetter { body })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_llm::mock::MockLlm;
    use careerai_profile::schema::{Personal, Profile};
    use chrono::Utc;
    use tempfile::tempdir;

    fn fixture_cfg() -> LlmConfig {
        LlmConfig {
            provider: String::new(),
            model: String::new(),
            tailor_model: "claude".into(),
            cover_letter_model: "claude".into(),
            filter_model: String::new(),
            parse_resume_model: String::new(),
            cache_dir: String::new(),
            api_base_url: None,
            api_key: None,
            max_retries: 3,
            timeout_seconds: 120,
            prompt_version: "tailor.v1".into(),
            anthropic_prompt_cache: true,
            backend: careerai_core::config::BackendChoice::default(),
            ..Default::default()
        }
    }

    fn fixture_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "A".into(),
                ..Default::default()
            },
            summary: "x".into(),
            ..Default::default()
        }
    }

    fn fixture_listing() -> Listing {
        Listing {
            id: "l".into(),
            source: "s".into(),
            external_id: "e".into(),
            title: "t".into(),
            company: "c".into(),
            location: None,
            url: "u".into(),
            description: "d".into(),
            raw_json: None,
            state: "shortlisted".into(),
            score: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn accepts_short_canned_response() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        // The cover_letter prompt_version is `<cfg.prompt_version>+cover_letter`.
        let llm = MockLlm::with_fixture("tailor.v1+cover_letter", "short body text");
        let letter = draft(
            &llm,
            &fixture_profile(),
            &fixture_listing(),
            &fixture_cfg(),
            &cache,
            "profile-hash-fixture",
        )
        .await
        .unwrap();
        assert_eq!(letter.body, "short body text");
    }

    #[tokio::test]
    async fn rejects_response_over_word_cap() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        // 351 whitespace-split words.
        let long: String = (0..351).map(|_| "word").collect::<Vec<_>>().join(" ");
        let llm = MockLlm::with_fixture("tailor.v1+cover_letter", long);
        let err = draft(
            &llm,
            &fixture_profile(),
            &fixture_listing(),
            &fixture_cfg(),
            &cache,
            "profile-hash-fixture",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, TailorError::CoverLetterTooLong { words, cap } if words == 351 && cap == 350),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn second_call_hits_cache() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let llm = MockLlm::with_fixture("tailor.v1+cover_letter", "hello");
        let _ = draft(
            &llm,
            &fixture_profile(),
            &fixture_listing(),
            &fixture_cfg(),
            &cache,
            "profile-hash-fixture",
        )
        .await
        .unwrap();

        // Second call: use an LLM with NO fixture — the cache must satisfy
        // the request. If we're still hitting the LLM, this will error out
        // with `Upstream`.
        let llm2 = MockLlm::new();
        let again = draft(
            &llm2,
            &fixture_profile(),
            &fixture_listing(),
            &fixture_cfg(),
            &cache,
            "profile-hash-fixture",
        )
        .await
        .unwrap();
        assert_eq!(again.body, "hello");
    }
}
