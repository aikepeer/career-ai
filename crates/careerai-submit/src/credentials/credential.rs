use std::env;

use crate::error::{Result, SubmitError};

/// Service name used for keychain entries.
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

    pub(super) fn keyring_username(&self) -> String {
        format!("{}/{}", self.source, self.name)
    }

    pub(super) fn env_var_name(&self) -> String {
        format!(
            "CAREERAI_{}_{}",
            self.source.to_uppercase(),
            self.name.to_uppercase().replace('-', "_")
        )
    }
}

/// Load a credential. Tries the OS keychain first; if unavailable, falls
/// back to the env var documented per credential.
pub fn load(cred: &Credential) -> Result<String> {
    match keyring::Entry::new(SERVICE, &cred.keyring_username()) {
        Ok(entry) => match entry.get_password() {
            Ok(secret) => return Ok(secret),
            Err(keyring::Error::NoEntry) => {}
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

    let var = cred.env_var_name();
    match env::var(&var) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(SubmitError::SourceDisabled(format!(
            "no credential for {}/{} — store in keychain (service '{SERVICE}', user '{}') or set ${var} (env)",
            cred.source,
            cred.name,
            cred.keyring_username(),
        ))),
    }
}

/// Store a credential in the keychain.
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
