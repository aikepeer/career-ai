//! LLM-backed resume → [`Profile`] extractor.
//!
//! The heuristic regex parser in [`crate::heuristic`] produces garbage on
//! the kind of free-form PDF text most resumes ship with — bullets bleed
//! into the next entry, titles get tagged with `Jan 2025`, education
//! shrinks to a year. This module replaces that path with a constrained
//! LLM call: we send the raw text plus a strict JSON-only schema preamble
//! and parse the response into a [`Profile`]. On schema-validation
//! failure we retry once with the validator error appended; two failures
//! produce [`ExtractError::MaxRetries`] so the caller can fall back.
//!
//! ## Cycle avoidance
//!
//! `careerai-llm` already depends on `careerai-profile` (for
//! `canonical_profile_hash`), so this crate can't depend on
//! `careerai-llm` directly. To still let callers share their `Llm`
//! implementation, this module defines the small [`LlmCaller`] async
//! trait — a one-method shim around the gateway's complete()
//! call. `careerai-llm` provides a blanket impl, so any `Llm` works as
//! an `LlmCaller` at the CLI/orchestration boundary.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};
use validator::Validate;

use crate::schema::Profile;

/// Default prompt version baked into the cache key. Bump this whenever the
/// prompt body or schema preamble changes meaningfully.
pub const DEFAULT_PROMPT_VERSION: &str = "profile_extract.v1";

/// Default model. Overridable via [`ExtractOptions::model`]. Picked to be
/// cheap and fast; the user can swap to Sonnet/Opus via config.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";

/// Tunable knobs for [`extract_profile_from_text`]. Kept out of
/// `careerai-core::config::LlmConfig` to avoid a `core ↔ profile`
/// dependency edge — the CLI translates between them.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub model: String,
    pub prompt_version: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Pass-through to the provider. When true, the schema preamble is
    /// sent in the cacheable `profile_block` segment so Anthropic prompt
    /// caching can reuse it across multiple extracts.
    pub cache_schema: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            prompt_version: DEFAULT_PROMPT_VERSION.to_string(),
            temperature: 0.0,
            max_tokens: 4096,
            cache_schema: true,
        }
    }
}

/// Small request DTO mirroring `careerai-llm::LlmRequest` minus the
/// fields we don't need at this boundary. Keeps the trait independent
/// of the gateway crate.
#[derive(Debug, Clone)]
pub struct ExtractRequest {
    pub system: String,
    /// The cacheable schema preamble; sent verbatim and tagged for
    /// Anthropic prompt caching when `cache_schema` is set.
    pub profile_block: String,
    pub user: String,
    pub prompt_version: String,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub cache_schema: bool,
}

/// Provider-agnostic shim consumed by [`extract_profile_from_text`].
///
/// The async-trait erases the future so we can take this as
/// `&dyn LlmCaller`. `careerai-llm` provides a blanket impl over its
/// `Llm` trait, so callers don't normally implement this directly.
#[async_trait]
pub trait LlmCaller: Send + Sync {
    /// Issue a single completion. Implementations own their own retry
    /// policy; this module retries once on schema-validation failure
    /// (separate concern — re-prompting, not transient I/O).
    async fn call(&self, req: &ExtractRequest) -> Result<String, String>;
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("llm call failed: {0}")]
    LlmCall(String),

    #[error("response is not valid JSON: {0}")]
    ParseJson(String),

    #[error("response failed schema validation: {0}")]
    SchemaValidate(String),

    #[error("schema validation failed twice; giving up")]
    MaxRetries,
}

/// Build the system message. Static so it's easy to snapshot.
#[must_use]
pub fn build_system_message() -> String {
    "You are a resume parser. Extract structured data from the resume \
text the user provides. You MUST output a single JSON object that \
matches the supplied schema, with no prose, no markdown fences, and no \
trailing commentary. Never invent experience, titles, dates, or \
employers — if a field is unknown, use an empty string for scalars and \
an empty array for lists. Dates must be `YYYY-MM`, `YYYY`, the literal \
`Present`, or empty."
        .to_string()
}

/// The schema preamble sent in the cacheable profile_block. Hardcoded
/// here so prompt-cache hit rate doesn't depend on serde introspection
/// (which can drift with the struct definition).
#[must_use]
pub fn build_schema_preamble() -> String {
    r#"OUTPUT SCHEMA (JSON):
{
  "personal": {
    "name": "string (required)",
    "email": "string",
    "phone": "string",
    "location": "string",
    "links": {
      "github": "string (URL)",
      "linkedin": "string (URL)",
      "portfolio": "string (URL)"
    }
  },
  "summary": "string",
  "skills": {
    "languages": ["string", ...],
    "frameworks": ["string", ...],
    "tools": ["string", ...]
  },
  "experience": [
    {
      "title": "string",
      "company": "string",
      "location": "string",
      "start": "YYYY-MM | YYYY | empty",
      "end":   "YYYY-MM | YYYY | Present | empty",
      "bullets": ["string", ...]
    }
  ],
  "education": [
    {
      "degree": "string",
      "institution": "string",
      "start": "YYYY | YYYY-MM | empty",
      "end":   "YYYY | YYYY-MM | empty"
    }
  ],
  "projects": [
    {
      "name": "string",
      "url": "string (URL or empty)",
      "bullets": ["string", ...]
    }
  ]
}

RULES:
- Output a single JSON object matching the schema. No prose, no markdown.
- Never invent. Use "" for unknown scalars, [] for unknown lists.
- Each experience entry's `title` must be a job title, not a date.
  If the resume only shows a date and no title, leave `title` empty.
- Each education entry's `institution` must be the school name; the
  `degree` is the qualification (e.g. "B.Tech Computer Science").
- Bullets are individual achievement lines; do not concatenate.
"#
    .to_string()
}

/// Build the user message wrapping the resume text.
#[must_use]
pub fn build_user_message(resume_text: &str) -> String {
    let trimmed = resume_text.trim();
    format!("RESUME TEXT (verbatim, may contain OCR artifacts):\n---\n{trimmed}\n---\n\nReturn the JSON object now.")
}

/// Build the full extraction request. Pure function — easy to snapshot.
#[must_use]
pub fn build_request(resume_text: &str, opts: &ExtractOptions) -> ExtractRequest {
    ExtractRequest {
        system: build_system_message(),
        profile_block: build_schema_preamble(),
        user: build_user_message(resume_text),
        prompt_version: opts.prompt_version.clone(),
        model: opts.model.clone(),
        temperature: opts.temperature,
        max_tokens: opts.max_tokens,
        cache_schema: opts.cache_schema,
    }
}

/// Strip a leading ```json (or ```) fence and matching trailing ``` if
/// present. Tolerates whitespace and a trailing newline. Idempotent on
/// already-clean input.
#[must_use]
pub fn strip_code_fences(text: &str) -> String {
    let t = text.trim();
    let inner = if let Some(stripped) = t.strip_prefix("```json") {
        stripped
    } else if let Some(stripped) = t.strip_prefix("```") {
        stripped
    } else {
        return t.to_string();
    };
    let inner = inner.trim_start_matches('\n');
    if let Some(stripped) = inner.strip_suffix("```") {
        stripped.trim_end_matches('\n').to_string()
    } else {
        inner.to_string()
    }
}

/// Parse the LLM response text into a [`Profile`] and validate. Used by
/// [`extract_profile_from_text`] and exposed for tests / fallback paths.
///
/// # Errors
///
/// Returns [`ExtractError::ParseJson`] if the text isn't a JSON object
/// matching the [`Profile`] schema, or [`ExtractError::SchemaValidate`]
/// if the validator rejects it (e.g. empty `personal.name`).
pub fn parse_and_validate(response_text: &str) -> Result<Profile, ExtractError> {
    let cleaned = strip_code_fences(response_text);

    // First parse into Value so we can give a richer error than serde's
    // own "missing field" output, which often points at the wrong line.
    let value: Value =
        serde_json::from_str(&cleaned).map_err(|e| ExtractError::ParseJson(e.to_string()))?;

    let profile = Profile::deserialize(value)
        .map_err(|e| ExtractError::ParseJson(format!("schema mismatch: {e}")))?;

    Validate::validate(&profile).map_err(|e| ExtractError::SchemaValidate(e.to_string()))?;

    Ok(profile)
}

/// Extract a [`Profile`] from raw resume text via an LLM.
///
/// Retries exactly once on parse / schema validation failure, with the
/// previous error appended to the user message so the model can self-
/// correct. After the second failure, returns [`ExtractError::MaxRetries`].
///
/// # Errors
///
/// - [`ExtractError::LlmCall`] when the underlying [`LlmCaller`] returns
///   an error on the first attempt (we don't retry transport errors here
///   — the gateway already does).
/// - [`ExtractError::MaxRetries`] when both attempts fail to parse +
///   validate.
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
            // Transport failure on retry isn't a schema-validation
            // exhaustion — surface it as `LlmCall` so callers can
            // distinguish upstream I/O from grammar mismatches.
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Minimal test double for [`LlmCaller`] — returns a queue of
    /// canned responses in order. Errors out if the queue runs dry.
    struct ScriptedLlm {
        queue: Mutex<Vec<Result<String, String>>>,
    }

    impl ScriptedLlm {
        fn new(responses: Vec<Result<String, String>>) -> Self {
            Self {
                queue: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LlmCaller for ScriptedLlm {
        async fn call(&self, _req: &ExtractRequest) -> Result<String, String> {
            let mut q = self.queue.lock().unwrap();
            if q.is_empty() {
                return Err("scripted queue empty".to_string());
            }
            q.remove(0)
        }
    }

    fn valid_response() -> &'static str {
        r#"{
  "personal": {"name": "Alice Kumar", "email": "alice@example.com", "phone": "", "location": "Delhi", "links": {"github": "", "linkedin": "", "portfolio": ""}},
  "summary": "Builder of systems.",
  "skills": {"languages": ["Rust"], "frameworks": [], "tools": []},
  "experience": [{"title": "Senior Engineer", "company": "Acme", "location": "Remote", "start": "2022-01", "end": "Present", "bullets": ["Shipped things"]}],
  "education": [{"degree": "B.Tech", "institution": "IIT Delhi", "start": "2015", "end": "2019"}],
  "projects": []
}"#
    }

    #[test]
    fn strip_fences_handles_json_marker() {
        let s = "```json\n{\"a\":1}\n```";
        assert_eq!(strip_code_fences(s), "{\"a\":1}");
    }

    #[test]
    fn strip_fences_handles_bare_marker() {
        let s = "```\n{\"a\":1}\n```";
        assert_eq!(strip_code_fences(s), "{\"a\":1}");
    }

    #[test]
    fn strip_fences_passthrough_when_clean() {
        let s = "{\"a\":1}";
        assert_eq!(strip_code_fences(s), "{\"a\":1}");
    }

    #[test]
    fn parse_and_validate_accepts_minimal_profile() {
        let p = parse_and_validate(valid_response()).unwrap();
        assert_eq!(p.personal.name, "Alice Kumar");
        assert_eq!(p.experience.len(), 1);
    }

    #[test]
    fn parse_and_validate_rejects_empty_name() {
        let bad = r#"{"personal":{"name":""},"summary":"","skills":{"languages":[],"frameworks":[],"tools":[]},"experience":[],"education":[],"projects":[]}"#;
        let err = parse_and_validate(bad).unwrap_err();
        assert!(matches!(err, ExtractError::SchemaValidate(_)));
    }

    #[test]
    fn parse_and_validate_rejects_non_json() {
        let err = parse_and_validate("not json at all").unwrap_err();
        assert!(matches!(err, ExtractError::ParseJson(_)));
    }

    #[tokio::test]
    async fn extract_succeeds_on_first_attempt() {
        let llm = ScriptedLlm::new(vec![Ok(valid_response().to_string())]);
        let opts = ExtractOptions::default();
        let p = extract_profile_from_text("resume text", &llm, &opts)
            .await
            .unwrap();
        assert_eq!(p.personal.name, "Alice Kumar");
    }

    #[tokio::test]
    async fn extract_recovers_on_retry() {
        // First response is malformed, second is valid.
        let llm = ScriptedLlm::new(vec![
            Ok("not json".to_string()),
            Ok(valid_response().to_string()),
        ]);
        let p = extract_profile_from_text("resume text", &llm, &ExtractOptions::default())
            .await
            .unwrap();
        assert_eq!(p.personal.name, "Alice Kumar");
    }

    #[tokio::test]
    async fn extract_gives_up_after_two_failures() {
        let llm = ScriptedLlm::new(vec![
            Ok("not json".to_string()),
            Ok("still not json".to_string()),
        ]);
        let err = extract_profile_from_text("resume text", &llm, &ExtractOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ExtractError::MaxRetries));
    }

    #[tokio::test]
    async fn extract_surfaces_first_call_transport_error() {
        let llm = ScriptedLlm::new(vec![Err("rate limited".to_string())]);
        let err = extract_profile_from_text("resume text", &llm, &ExtractOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ExtractError::LlmCall(_)));
    }

    #[test]
    fn request_build_is_deterministic() {
        let opts = ExtractOptions::default();
        let r1 = build_request("hello", &opts);
        let r2 = build_request("hello", &opts);
        assert_eq!(r1.system, r2.system);
        assert_eq!(r1.profile_block, r2.profile_block);
        assert_eq!(r1.user, r2.user);
    }
}
