use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};
use validator::Validate;

use crate::schema::Profile;

use super::build::build_request;
use super::types::{ExtractError, ExtractOptions, LlmCaller};

/// Strip a leading ```json (or ```) fence and matching trailing ``` if
/// present. Tolerates whitespace and a trailing newline. Idempotent on
/// already-clean input.
#[must_use]
pub fn strip_code_fences(text: &str) -> String {
    let t = text.trim();
    let unboxed = if let Some(first_brace) = t.find('{') {
        if let Some(last_brace) = t.rfind('}') {
            if last_brace >= first_brace {
                &t[first_brace..=last_brace]
            } else {
                t
            }
        } else {
            t
        }
    } else {
        t
    };
    unboxed.trim().to_string()
}

/// Parse the LLM response text into a [`Profile`] and validate.
pub fn parse_and_validate(response_text: &str) -> Result<Profile, ExtractError> {
    let cleaned = strip_code_fences(response_text);

    let value: Value =
        serde_json::from_str(&cleaned).map_err(|e| ExtractError::ParseJson(e.to_string()))?;

    let profile =
        Profile::deserialize(value).map_err(|e| ExtractError::SchemaDeserialize(e.to_string()))?;

    Validate::validate(&profile).map_err(|e| ExtractError::SchemaValidate(e.to_string()))?;

    Ok(profile)
}

/// Extract a [`Profile`] from raw resume text via an LLM.
///
/// Retries exactly once on parse / schema validation failure, with the
/// previous error appended to the user message so the model can self-
/// correct. After the second failure, returns [`ExtractError::MaxRetries`].
pub async fn extract_profile_from_text(
    text: &str,
    llm: &dyn LlmCaller,
    opts: &ExtractOptions,
) -> Result<Profile, ExtractError> {
    let mut req = build_request(text, opts);

    let first_text = llm.call(&req).await.map_err(ExtractError::LlmCall)?;

    match parse_and_validate(&first_text) {
        Ok(profile) => {
            debug!(target: "profile.llm_extract", "first attempt succeeded");
            return Ok(profile);
        }
        Err(e) => {
            warn!(
                target: "profile.llm_extract",
                error = %e,
                "first attempt failed; retrying with feedback"
            );
            req.user = format!(
                "{}\n\nYour previous response could not be parsed:\n{}\n\n\
                 Return ONLY a single JSON object matching the schema, no \
                 prose, no markdown fences.",
                req.user, e
            );
        }
    }

    let second_text = match llm.call(&req).await {
        Ok(t) => t,
        Err(e) => {
            warn!(target: "profile.llm_extract", error = %e, "retry call failed");
            return Err(ExtractError::LlmCall(e));
        }
    };

    match parse_and_validate(&second_text) {
        Ok(profile) => Ok(profile),
        Err(e) => {
            warn!(target: "profile.llm_extract", error = %e, "retry parse failed");
            Err(ExtractError::MaxRetries)
        }
    }
}
