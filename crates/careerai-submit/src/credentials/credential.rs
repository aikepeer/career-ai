use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

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

    pub fn keyring_username(&self) -> String {
        format!("{}/{}", self.source, self.name)
    }

    pub fn env_var_name(&self) -> String {
        format!(
            "CAREERAI_{}_{}",
            self.source.to_uppercase(),
            self.name.to_uppercase().replace('-', "_")
        )
    }
}

fn credentials_file_path() -> PathBuf {
    careerai_core::paths::database_path(&careerai_core::paths::resolve_root_env())
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("credentials.json")
}

fn load_from_file(username: &str) -> Option<String> {
    let path = credentials_file_path();
    let text = fs::read_to_string(&path).ok()?;
    let map: HashMap<String, String> = serde_json::from_str(&text).ok()?;
    map.get(username).cloned()
}

fn save_to_file(username: &str, secret: &str) -> Result<()> {
    let path = credentials_file_path();
    let mut map: HashMap<String, String> = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    map.insert(username.to_string(), secret.to_string());
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| SubmitError::SourceDisabled(format!("serialize credentials: {e}")))?;
    write_secrets_file(&path, json.as_bytes())
}

/// Atomically write the plaintext secrets file with owner-only
/// permissions. The file is created with mode 0600 up front (via
/// `OpenOptionsExt::mode`) — never write-then-chmod, which leaves a
/// window where the file exists world-readable under the process umask.
/// Writes go through a temp file + rename so a crash mid-write cannot
/// leave a truncated secrets file behind.
fn write_secrets_file(path: &std::path::Path, contents: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        // Temp file: create with 0600 atomically, in the same dir so the
        // rename below is same-filesystem.
        let tmp_path = path.with_extension("json.tmp");
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        opts.mode(0o600);
        let mut f = opts
            .open(&tmp_path)
            .map_err(|e| SubmitError::SourceDisabled(format!("open credentials tmp file: {e}")))?;
        f.write_all(contents)
            .and_then(|()| f.sync_all())
            .map_err(|e| SubmitError::SourceDisabled(format!("write credentials tmp file: {e}")))?;
        drop(f);
        fs::rename(&tmp_path, path)
            .map_err(|e| SubmitError::SourceDisabled(format!("rename credentials file: {e}")))?;
        // Cover the case where an old file already existed with loose perms.
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| SubmitError::SourceDisabled(format!("set credentials file perms: {e}")))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
            .map_err(|e| SubmitError::SourceDisabled(format!("write credentials file: {e}")))
    }
}

fn delete_from_file(username: &str) {
    let path = credentials_file_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut map): std::result::Result<HashMap<String, String>, _> = serde_json::from_str(&text)
    else {
        return;
    };
    if map.remove(username).is_none() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = write_secrets_file(&path, json.as_bytes());
    }
}

/// Load a credential. Tries the OS keychain first, then env var, then data/credentials.json.
pub fn load(cred: &Credential) -> Result<String> {
    // 1) Try keychain
    if let Ok(entry) = keyring::Entry::new(SERVICE, &cred.keyring_username()) {
        if let Ok(secret) = entry.get_password() {
            if !secret.is_empty() {
                return Ok(secret);
            }
        }
    }

    // 2) Try env var
    let var = cred.env_var_name();
    if let Ok(v) = env::var(&var) {
        if !v.is_empty() {
            return Ok(v);
        }
    }

    // 3) Try local credentials file
    if let Some(secret) = load_from_file(&cred.keyring_username()) {
        if !secret.is_empty() {
            return Ok(secret);
        }
    }

    Err(SubmitError::SourceDisabled(format!(
        "no credential for {}/{} — store in keychain (service '{SERVICE}', user '{}') or set ${var} (env)",
        cred.source,
        cred.name,
        cred.keyring_username(),
    )))
}

/// Store a credential in keychain and local credentials file.
/// Returns Ok only if at least one storage backend succeeded.
pub fn store(cred: &Credential, secret: &str) -> Result<()> {
    let mut keychain_ok = false;
    if let Ok(entry) = keyring::Entry::new(SERVICE, &cred.keyring_username()) {
        match entry.set_password(secret) {
            Ok(()) => keychain_ok = true,
            Err(e) => tracing::warn!(
                target: "credentials",
                source = %cred.source,
                name = %cred.name,
                error = %e,
                "keychain store failed, falling back to file"
            ),
        }
    }
    let file_ok = save_to_file(&cred.keyring_username(), secret).is_ok();
    if !keychain_ok && !file_ok {
        return Err(SubmitError::SourceDisabled(format!(
            "failed to store credential {}/{}: both keychain and file write failed",
            cred.source, cred.name
        )));
    }
    tracing::info!(
        target: "credentials",
        source = %cred.source,
        name = %cred.name,
        "stored credential"
    );
    Ok(())
}

/// Delete a credential from the keychain and local credentials file. No-op if it doesn't exist.
pub fn delete(cred: &Credential) -> Result<()> {
    // Always remove the plaintext copy — store() writes to both backends.
    delete_from_file(&cred.keyring_username());

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
