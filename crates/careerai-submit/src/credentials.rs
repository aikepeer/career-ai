//! OS-keychain-backed credential store for browser submitters.
//!
//! Primary path: `keyring` crate (Linux Secret Service / macOS Keychain
//! / Windows Credential Manager). Fallback for CI / containers without
//! a keyring daemon: env vars prefixed `CAREERAI_<SOURCE>_<NAME>`. The
//! fallback is documented; production single-user installs always use
//! the keychain.
//!
//! Secrets are never logged. Errors include the credential *name* but
//! not the *value*. Tests assert this via captured tracing events in
//! Wave 3.

use std::env;

use crate::error::{Result, SubmitError};

/// Service name used for keychain entries. Picked once and never
/// changed across versions — renaming this string would orphan every
/// existing credential.
///
/// `pub` so the `careerai cookies refresh` CLI writes to the same
/// keychain entry the daemon reads. A drifted duplicate string would
/// produce a stale-cookie bug that's tedious to diagnose.
pub const SERVICE: &str = "career-ai";

/// A typed credential reference. Compose via `Credential::for_source(src, name)`.
#[derive(Debug, Clone)]
pub struct Credential {
    pub source: String,
    pub name: String,
}

impl Credential {
    pub fn for_source(source: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            name: name.into(),
        }
    }

    fn keyring_username(&self) -> String {
        format!("{}/{}", self.source, self.name)
    }

    fn env_var_name(&self) -> String {
        // CAREERAI_LINKEDIN_LI_AT for source=linkedin, name=li_at.
        format!(
            "CAREERAI_{}_{}",
            self.source.to_uppercase(),
            self.name.to_uppercase().replace('-', "_")
        )
    }
}

/// Load a credential. Tries the OS keychain first; if the keychain is
/// unavailable (Linux without Secret Service, container), falls back
/// to the env var documented per credential. Missing credentials
/// surface as `SubmitError::SourceDisabled` with an actionable message
/// telling the operator how to set it.
pub fn load(cred: &Credential) -> Result<String> {
    // Try keychain first.
    match keyring::Entry::new(SERVICE, &cred.keyring_username()) {
        Ok(entry) => match entry.get_password() {
            Ok(secret) => return Ok(secret),
            Err(keyring::Error::NoEntry) => {
                // Fall through to env var.
            }
            Err(e) => {
                tracing::warn!(
                    target: "credentials",
                    source = %cred.source,
                    name = %cred.name,
                    error = %e,
                    "keychain read failed; falling back to env var"
                );
            }
        },
        Err(e) => {
            tracing::warn!(
                target: "credentials",
                source = %cred.source,
                name = %cred.name,
                error = %e,
                "keychain unavailable; falling back to env var"
            );
        }
    }

    // Env var fallback.
    let var = cred.env_var_name();
    match env::var(&var) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(SubmitError::SourceDisabled(format!(
            "no credential for {}/{} — store in keychain (service '{}', user '{}') or set ${} (env)",
            cred.source,
            cred.name,
            SERVICE,
            cred.keyring_username(),
            var
        ))),
    }
}

pub fn cookie_expiry(provider: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let key = match provider {
        "linkedin" => "li_at",
        // Naukri cookies are opaque session strings, not JWTs.
        _ => return None,
    };
    let cred = Credential::for_source(provider, key);
    let token = load(&cred).ok()?;
    parse_jwt_exp(&token)
}

/// Parse the `exp` claim out of a JWT string. Extracted so unit tests can
/// exercise the production parser directly (without touching the OS
/// keyring). `cookie_expiry` is then a 4-line wrapper around the
/// keyring-read + this function.
///
/// Returns `None` when:
///   - the token isn't JWT-shaped (3 dot-separated segments),
///   - the middle segment isn't valid base64url,
///   - the decoded payload isn't valid JSON,
///   - or the JSON has no numeric `exp` field.
///
/// **Never logs the input token** — the JWT is a session-equivalent
/// secret. Errors are reduced to `None` deliberately; the caller
/// (`collect_cookie_warnings`) compensates by surfacing a different
/// warning when the cookie is present in the keyring but unparseable.
#[must_use]
pub fn parse_jwt_exp(token: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use base64::Engine;

    // JWT format: header.payload.signature (3 base64url segments).
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _sig = parts.next()?;
    if parts.next().is_some() {
        // 4+ segments → not a JWT.
        return None;
    }

    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let exp = json.get("exp")?.as_i64()?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(exp, 0)
}

/// Wrapper around `cookie_expiry` returning `(exp - now)`. Negative
/// duration means the cookie has already expired. `None` propagates the
/// "no JWT / no exp claim / not in keyring" cases.
#[must_use]
pub fn cookie_remaining(provider: &str) -> Option<chrono::Duration> {
    let exp = cookie_expiry(provider)?;
    Some(exp - chrono::Utc::now())
}

/// Cookie health classification used by `careerai digest` to surface
/// actionable warnings. Splits the `cookie_remaining`-returns-`None`
/// case (silent today) into "absent" vs "present-but-unparseable" so
/// the operator gets a different fix instruction in each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieHealth {
    /// No keyring entry. Operator never ran `careerai cookies refresh`.
    NotStored,
    /// Keyring entry exists but couldn't decode `exp` from the JWT.
    /// Either the token isn't a JWT or it's been corrupted.
    Unparseable,
    /// `now > exp`. The duration is positive (`now - exp`).
    Expired(chrono::Duration),
    /// `exp - now < 48h`. The duration is positive (`exp - now`).
    ExpiringSoon(chrono::Duration),
    /// Healthy: more than 48h until expiry.
    Healthy(chrono::Duration),
}

/// Diagnose the LinkedIn `li_at` cookie. Returns `CookieHealth` so the
/// CLI can surface the right warning instead of silently dropping every
/// failure mode into `None`.
///
/// Read-only on the keyring; never logs the token value. Naukri (and
/// other opaque-cookie providers) currently return `NotStored` when
/// absent and `Unparseable` when present (no JWT shape) — the digest
/// caller skips Naukri entirely.
#[must_use]
pub fn cookie_health(provider: &str) -> CookieHealth {
    let key = match provider {
        "linkedin" => "li_at",
        _ => return CookieHealth::NotStored,
    };
    let cred = Credential::for_source(provider, key);
    let Ok(token) = load(&cred) else {
        return CookieHealth::NotStored;
    };
    let Some(exp) = parse_jwt_exp(&token) else {
        return CookieHealth::Unparseable;
    };
    let now = chrono::Utc::now();
    if exp <= now {
        return CookieHealth::Expired(now - exp);
    }
    let remaining = exp - now;
    if remaining < chrono::Duration::hours(48) {
        CookieHealth::ExpiringSoon(remaining)
    } else {
        CookieHealth::Healthy(remaining)
    }
}

/// Store a credential in the keychain. Used by an `init` / `setup`
/// path that the user runs once (e.g. `careerai linkedin login`).
/// Falls back to a clear error when no keychain is available — we
/// never write secrets to the filesystem implicitly.
pub fn store(cred: &Credential, secret: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, &cred.keyring_username()).map_err(|e| {
        SubmitError::SourceDisabled(format!(
            "keychain unavailable for {}/{}: {e} — set the env var instead: {}",
            cred.source,
            cred.name,
            cred.env_var_name()
        ))
    })?;
    entry.set_password(secret).map_err(|e| {
        SubmitError::SourceDisabled(format!(
            "keychain write failed for {}/{}: {e}",
            cred.source, cred.name
        ))
    })?;
    tracing::info!(
        target: "credentials",
        source = %cred.source,
        name = %cred.name,
        "stored credential in keychain"
    );
    Ok(())
}

/// Delete a credential from the keychain. No-op if it doesn't exist.
pub fn delete(cred: &Credential) -> Result<()> {
    let Ok(entry) = keyring::Entry::new(SERVICE, &cred.keyring_username()) else {
        return Ok(());
    };
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(SubmitError::SourceDisabled(format!(
            "keychain delete failed for {}/{}: {e}",
            cred.source, cred.name
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn env_var_name_uppercases_with_underscores() {
        let c = Credential::for_source("linkedin", "li_at");
        assert_eq!(c.env_var_name(), "CAREERAI_LINKEDIN_LI_AT");
        let c = Credential::for_source("indeed", "session-cookie");
        assert_eq!(c.env_var_name(), "CAREERAI_INDEED_SESSION_COOKIE");
    }

    #[test]
    fn keyring_username_is_namespaced_per_source() {
        let c = Credential::for_source("linkedin", "li_at");
        assert_eq!(c.keyring_username(), "linkedin/li_at");
    }

    /// Build a synthetic JWT (no signing) for `cookie_expiry` tests.
    /// Returns `header.payload.sig` where `payload.exp = exp_secs`.
    fn fake_jwt_with_exp(exp_secs: i64) -> String {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload_json = format!(r#"{{"sub":"test","exp":{exp_secs}}}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"unverified");
        format!("{header}.{payload}.{sig}")
    }

    /// `parse_jwt_exp` decodes the `exp` claim from a JWT regardless of
    /// signature validity. Exercising the production parser directly
    /// (rather than reproducing the inline logic) means a refactor that
    /// breaks the JWT split or base64 decode can't pass tests.
    #[test]
    fn parse_jwt_exp_decodes_valid_jwt() {
        let exp = 2_000_000_000_i64; // 2033-05-18 UTC
        let jwt = fake_jwt_with_exp(exp);
        let parsed = parse_jwt_exp(&jwt).expect("valid JWT must decode");
        assert_eq!(parsed.timestamp(), exp);
    }

    #[test]
    fn parse_jwt_exp_rejects_non_jwt_shapes() {
        // Not enough dots.
        assert!(parse_jwt_exp("opaque-cookie").is_none());
        assert!(parse_jwt_exp("a.b").is_none());
        // Too many dots.
        assert!(parse_jwt_exp("a.b.c.d").is_none());
    }

    #[test]
    fn parse_jwt_exp_rejects_unparseable_payload() {
        use base64::Engine;
        // Valid JWT shape but middle segment isn't base64url.
        assert!(parse_jwt_exp("header.notbase64!@#.sig").is_none());
        // base64url decodes but isn't JSON.
        let bad_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not json");
        assert!(parse_jwt_exp(&format!("h.{bad_payload}.s")).is_none());
    }

    #[test]
    fn parse_jwt_exp_rejects_payload_without_exp_claim() {
        use base64::Engine;
        let json = r#"{"sub":"test"}"#;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());
        assert!(parse_jwt_exp(&format!("h.{payload}.s")).is_none());
    }

    #[test]
    fn cookie_expiry_short_circuits_for_non_linkedin_provider() {
        // Naukri cookies are opaque session strings, not JWTs.
        // cookie_expiry must short-circuit before hitting the keyring.
        assert!(cookie_expiry("naukri").is_none());
        assert!(cookie_expiry("indeed").is_none());
        assert_eq!(cookie_health("naukri"), CookieHealth::NotStored);
        assert_eq!(cookie_health("indeed"), CookieHealth::NotStored);
    }

    #[test]
    fn cookie_expiry_handles_linkedin_lookup_without_panicking() {
        // Note: the developer's real OS keyring may or may not contain
        // a `career-ai`/`linkedin/li_at` entry. Asserting None would
        // fail on a machine where the operator has stored a real
        // cookie. Instead, only assert the call paths return — the
        // public API must not panic regardless of keyring state.
        let _ = cookie_expiry("linkedin");
        let _ = cookie_remaining("linkedin");
        let _ = cookie_health("linkedin");
    }

    #[test]
    fn missing_credential_error_is_actionable() {
        // Use an obviously-absent credential. Don't touch real keychain
        // entries — the test name space is `careerai-test-<random>` is
        // overkill for this; just pick a name no one would ever use.
        let c = Credential::for_source("__nonexistent_test_source__", "__nope__");
        let err = load(&c).unwrap_err();
        let msg = err.to_string();
        // Error must tell the operator BOTH paths to set it.
        assert!(msg.contains("keychain"), "got: {msg}");
        assert!(msg.contains("CAREERAI_"), "got: {msg}");
        // Error must NOT contain a literal value (we never had one).
        assert!(!msg.contains("password"), "got: {msg}");
    }
}
