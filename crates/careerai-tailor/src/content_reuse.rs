//! Domain/role classification and cover-letter reuse from the content
//! library (C4).
//!
//! When tailoring a new listing, we classify the listing title into a
//! (domain, role) pair. If the content library already has a cover letter
//! for that domain+role, it is reused directly — skipping the LLM call
//! for cover-letter drafting.

use careerai_db::queries;
use sqlx::SqlitePool;

/// Classify a listing title into a (domain, role) pair using keyword
/// matching. Returns ("general", "engineer") as fallback.
pub fn classify_domain(title: &str) -> (String, String) {
    let lower = title.to_lowercase();

    // Domain detection — check in priority order.
    let domain = if lower.contains("ml")
        || lower.contains("machine learning")
        || lower.contains("deep learning")
        || lower.contains("ai engineer")
        || lower.contains("data scientist")
        || lower.contains("pytorch")
        || lower.contains("tensorrt")
        || lower.contains("llm")
    {
        "ml"
    } else if lower.contains("embedded")
        || lower.contains("firmware")
        || lower.contains("bsp")
        || lower.contains("yocto")
        || lower.contains("rtos")
        || lower.contains("bootloader")
        || lower.contains("device driver")
        || lower.contains("kernel")
        || lower.contains("soc")
    {
        "embedded"
    } else if lower.contains("frontend")
        || lower.contains("front-end")
        || lower.contains("react")
        || lower.contains("vue")
        || lower.contains("angular")
    {
        "frontend"
    } else if lower.contains("backend")
        || lower.contains("back-end")
        || lower.contains("api")
        || lower.contains("microservice")
        || lower.contains("distributed system")
    {
        "backend"
    } else if lower.contains("devops")
        || lower.contains("sre")
        || lower.contains("platform engineer")
        || lower.contains("infrastructure")
        || lower.contains("kubernetes")
    {
        "devops"
    } else if lower.contains("fullstack")
        || lower.contains("full stack")
        || lower.contains("full-stack")
    {
        "fullstack"
    } else if lower.contains("security")
        || lower.contains("crypto")
        || lower.contains("vulnerability")
    {
        "security"
    } else if lower.contains("robotics")
        || lower.contains("autonomous")
        || lower.contains("perception")
        || lower.contains("slam")
        || lower.contains("ros")
        || lower.contains("drone")
    {
        "robotics"
    } else if lower.contains("data") && (lower.contains("engineer") || lower.contains("pipeline")) {
        "data"
    } else if lower.contains("mobile") || lower.contains("ios") || lower.contains("android") {
        "mobile"
    } else {
        "general"
    };

    // Role detection.
    let role = if lower.contains("manager")
        || lower.contains("lead")
        || lower.contains("director")
        || lower.contains("head of")
        || lower.contains("principal")
    {
        "lead"
    } else if lower.contains("intern") {
        "intern"
    } else if lower.contains("architect") {
        "architect"
    } else if lower.contains("developer")
        || lower.contains("engineer")
        || lower.contains("programmer")
    {
        "engineer"
    } else if lower.contains("scientist") || lower.contains("researcher") {
        "researcher"
    } else {
        "engineer"
    };

    (domain.to_string(), role.to_string())
}

/// Try to reuse a cover letter from the content library for the given
/// domain + role. Returns `Ok(Some(text))` if a stored cover letter
/// exists for this candidate (profile_hash), `Ok(None)` if the library
/// has no entry, or `Err` on database failure.
///
/// R02: the previous version swallowed DB errors with `.ok().flatten()`,
/// treating a real DB failure as "no letter found" and silently falling
/// through to the LLM. The profile_hash parameter ensures a letter
/// stored for one candidate is never reused for a different candidate.
pub async fn try_reuse_cover_letter(
    pool: &SqlitePool,
    domain: &str,
    role: &str,
    profile_hash: &str,
) -> careerai_db::error::Result<Option<String>> {
    queries::fetch_cover_letter_for_domain(pool, domain, role, profile_hash).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_ml_engineer() {
        let (domain, role) = classify_domain("Senior ML Engineer");
        assert_eq!(domain, "ml");
        assert_eq!(role, "engineer");
    }

    #[test]
    fn classify_embedded_firmware() {
        let (domain, role) = classify_domain("Embedded Firmware Developer");
        assert_eq!(domain, "embedded");
        assert_eq!(role, "engineer");
    }

    #[test]
    fn classify_fullstack() {
        let (domain, role) = classify_domain("Full-stack Developer");
        assert_eq!(domain, "fullstack");
        assert_eq!(role, "engineer");
    }

    #[test]
    fn classify_robotics() {
        let (domain, role) = classify_domain("Robotics Perception Engineer");
        assert_eq!(domain, "robotics");
        assert_eq!(role, "engineer");
    }

    #[test]
    fn classify_fallback() {
        let (domain, role) = classify_domain("Product Manager");
        assert_eq!(domain, "general");
        assert_eq!(role, "lead");
    }
}
