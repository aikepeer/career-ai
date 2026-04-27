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
