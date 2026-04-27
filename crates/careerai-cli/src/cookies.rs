//! `careerai cookies refresh <provider>` — interactive helper for
//! capturing session cookies into the OS keyring.
//!
//! Walks the operator through:
//!   1. Open <provider> in your real browser and log in.
//!   2. Open DevTools → Application → Cookies.
//!   3. Find the named cookie ("li_at" for LinkedIn, the long opaque
//!      session value for Naukri).
//!   4. Paste the value here.
//!
//! Stores via `keyring` using the same service + username format that
//! `careerai_submit::credentials::load` reads:
//!   service  = "career-ai"
//!   username = "{source}/{key}"
//!
//! This ensures the daemon can immediately pick up the refreshed cookie
//! on the next apply tick without restarting.

use anyhow::{anyhow, Result};
use careerai_submit::credentials::SERVICE as KEYRING_SERVICE;

struct ProviderInfo {
    /// Keyring username in `{source}/{key}` format.
    keyring_username: &'static str,
    /// Human-readable cookie name shown in the prompt.
    cookie_name: &'static str,
    /// Where to find the cookie in the browser.
    where_to_find: &'static str,
}

fn provider_info(provider: &str) -> Option<ProviderInfo> {
    match provider {
        "linkedin" => Some(ProviderInfo {
            keyring_username: "linkedin/li_at",
            cookie_name: "li_at",
            where_to_find: "linkedin.com → DevTools → Application → Cookies → li_at",
        }),
        "naukri" => Some(ProviderInfo {
            keyring_username: "naukri/session_cookie",
            cookie_name: "naukri session cookie",
            where_to_find: "naukri.com → DevTools → Application → Cookies → \
                            look for the long opaque session value (nauk_at or similar)",
        }),
        _ => None,
    }
}

/// Refresh the session cookie for `provider` (linkedin or naukri).
///
/// Prompts the operator to paste the cookie value from their browser
/// DevTools, then writes it to the OS keyring. The running daemon and
/// future `careerai apply` invocations pick it up immediately.
pub fn refresh(provider: &str) -> Result<()> {
    let info = provider_info(provider)
        .ok_or_else(|| anyhow!("unknown provider '{provider}' (supported: linkedin, naukri)"))?;

    println!("Refreshing {provider} cookie ({}).", info.cookie_name);
    println!("  Where to find it: {}", info.where_to_find);

    // rpassword reads from the terminal with echo disabled — prevents the
    // pasted cookie from showing up in scrollback, screen-share, or shoulder-
    // surf. This is a session-equivalent secret and must be treated as one.
    let value = rpassword::prompt_password("  Paste cookie value (input hidden): ")
        .map_err(|e| anyhow!("read cookie failed: {e}"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("empty cookie value — aborting"));
    }

    let entry = keyring::Entry::new(KEYRING_SERVICE, info.keyring_username)
        .map_err(|e| anyhow!("keyring open failed: {e}"))?;
    entry
        .set_password(value)
        .map_err(|e| anyhow!("keyring write failed: {e}"))?;

    println!("Stored. Daemon will pick this up on next {provider} apply tick.");
    Ok(())
}
