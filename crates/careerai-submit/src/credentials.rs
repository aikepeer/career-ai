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

/// Decode the `exp` claim of a stored JWT-shaped cookie. LinkedIn's
/// `li_at` is a signed JWT; the middle segment (base64url-encoded JSON)
/// carries an `exp: <unix-seconds>` field. We DON'T verify the signature
/// here — we only inspect the timestamp. The cookie is already trusted
/// (it's our own session); the question is solely "is it about to
/// expire?".
///
/// Returns `None` when:
///   - the cookie isn't in the keyring (operator never refreshed),
///   - the cookie isn't JWT-shaped (Naukri's `nauk_at` is opaque),
///   - the middle segment doesn't decode as JSON,
///   - or the JSON has no numeric `exp` field.
///
/// Always read-only on the keyring; never logs the cookie value.
#[must_use]
pub fn cookie_expiry(provider: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use base64::Engine;

    let key = match provider {
        "linkedin" => "li_at",
        // Naukri cookies are opaque session strings, not JWTs.
        _ => return None,
    };
    let cred = Credential::for_source(provider, key);
    let token = load(&cred).ok()?;

    // JWT format: header.payload.signature  (3 base64url segments)
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _sig = parts.next()?; // presence-check only

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

    /// `cookie_expiry` ignores the signature segment and decodes the
    /// `exp` claim. We bypass the keyring path by exercising the JWT
    /// parser directly (test would otherwise depend on a real
    /// keyring entry).
    #[test]
    fn cookie_expiry_decodes_jwt_exp_claim() {
        use base64::Engine;
        let exp = 2_000_000_000_i64; // 2033-05-18 UTC
        let jwt = fake_jwt_with_exp(exp);

        // Reproduce the inner parsing logic — the public function reads
        // from the keyring, which we can't easily mock here.
        let payload_b64 = jwt.split('.').nth(1).unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let parsed = json.get("exp").unwrap().as_i64().unwrap();
        assert_eq!(parsed, exp);

        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(exp, 0).unwrap();
        assert_eq!(dt.timestamp(), exp);
    }

    #[test]
    fn cookie_expiry_returns_none_for_naukri_opaque_cookie() {
        // Naukri cookies are opaque session strings, not JWTs.
        // cookie_expiry must short-circuit before hitting the keyring.
        assert!(cookie_expiry("naukri").is_none());
        assert!(cookie_expiry("indeed").is_none());
    }

    #[test]
    fn cookie_expiry_returns_none_when_keyring_empty() {
        // No li_at present in test environment — must return None
        // gracefully rather than panicking.
        assert!(cookie_expiry("linkedin").is_none());
        assert!(cookie_remaining("linkedin").is_none());
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
