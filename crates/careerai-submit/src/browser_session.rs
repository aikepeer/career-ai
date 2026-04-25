//! Chromium browser session with stealth JS injection.
//!
//! Wraps `chromiumoxide::Browser` + a single `Page`. The stealth script
//! is injected via `Page::add_script_to_evaluate_on_new_document` so it
//! runs before any page JS, neutralizing the most common automation
//! detection vectors (`navigator.webdriver`, `chrome.runtime`,
//! `permissions.query`, plugins, WebGL vendor/renderer).
//!
//! Feature-gated on `browser`. The default build never pulls in
//! chromiumoxide so the M4 ATS HTTP path stays lean.
//!
//! # Runtime requirement
//!
//! A Chromium/Chrome binary must be on `PATH` (or resolvable by
//! `chromiumoxide`'s default detection). On Debian/Ubuntu:
//!
//! ```bash
//! sudo apt install chromium
//! ```
//!
//! If `BrowserSession::launch` returns an error about spawning the
//! browser, that's the missing dep.

#![cfg(feature = "browser")]

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig, HeadlessMode};
use chromiumoxide::cdp::browser_protocol::network::{CookieSameSite, SetCookieParams};
use chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams;
use chromiumoxide::page::{Page, ScreenshotParams};
use futures::StreamExt;
use sha2::{Digest, Sha256};

use crate::error::{Result, SubmitError};

const STEALTH_JS: &str = include_str!("../../../browser/stealth-v2.js");

/// SHA-256 of the bundled stealth script. Pinned by tests so any
/// in-tree change to `stealth-v2.js` becomes a deliberate visible
/// commit — not a silent update that breaks detection mitigation.
pub fn stealth_script_sha256() -> &'static str {
    static SHA: OnceLock<String> = OnceLock::new();
    SHA.get_or_init(|| {
        let mut h = Sha256::new();
        h.update(STEALTH_JS.as_bytes());
        hex::encode(h.finalize())
    })
}

/// Expected SHA of the in-tree `stealth-v2.js`. The test below pins
/// equality so any edit to the script forces a visible commit that
/// also bumps this constant — preventing silent mitigation regressions.
/// To intentionally update: change the script, run
/// `cargo test -p careerai-submit --features browser stealth_script_sha_matches_pin`,
/// copy the actual SHA from the failure into this constant, and commit
/// both changes together.
pub const EXPECTED_STEALTH_SHA: &str =
    "a3bbcc79d80f77099f4aa21611124653da6b9eecfc1c79fabedc76049a103c2f";

/// Configuration for a `BrowserSession`.
#[derive(Debug, Clone)]
pub struct BrowserSessionConfig {
    pub headless: bool,
    pub window_width: u32,
    pub window_height: u32,
    pub user_agent: String,
    pub launch_timeout_seconds: u64,
}

impl Default for BrowserSessionConfig {
    fn default() -> Self {
        Self {
            headless: true,
            window_width: 1280,
            window_height: 800,
            // A modern Chrome UA on Linux. Real submitters override
            // this from config in M5b.
            user_agent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
                .to_string(),
            launch_timeout_seconds: 30,
        }
    }
}

pub struct BrowserSession {
    browser: Browser,
    page: Page,
    _handler: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for BrowserSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSession").finish_non_exhaustive()
    }
}

impl BrowserSession {
    /// Launch a headless Chromium with stealth JS injected. The handler
    /// task drives the CDP event loop and is dropped with the session.
    ///
    /// Requires a Chromium/Chrome binary on `PATH`. See module docs
    /// for install hints.
    pub async fn launch(cfg: &BrowserSessionConfig) -> Result<Self> {
        // chromiumoxide 0.7's BrowserConfigBuilder has no dedicated
        // user_agent() setter — the UA is passed to the underlying
        // Chromium process as a CLI flag. This is functionally
        // equivalent (Chromium wires `--user-agent=...` into every
        // request) and documented at
        // https://peter.sh/experiments/chromium-command-line-switches/.
        let mut builder = BrowserConfig::builder()
            .window_size(cfg.window_width, cfg.window_height)
            .request_timeout(Duration::from_secs(cfg.launch_timeout_seconds))
            .arg(format!("--user-agent={}", cfg.user_agent));
        builder = if cfg.headless {
            builder.headless_mode(HeadlessMode::True)
        } else {
            builder.with_head()
        };
        let bcfg = builder
            .build()
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("BrowserConfig: {e}"))))?;

        let (browser, mut events) = Browser::launch(bcfg)
            .await
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("Browser::launch: {e}"))))?;

        // Drive CDP events so the connection stays alive.
        let handler = tokio::spawn(async move { while let Some(_ev) = events.next().await {} });

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("new_page: {e}"))))?;

        // Inject stealth BEFORE any navigation. add_script_to_evaluate_on_new_document
        // makes Chromium run the script as the very first thing on every
        // new page load, which is the only way to neutralize detection
        // probes that fire on document-start.
        let add_script = AddScriptToEvaluateOnNewDocumentParams::builder()
            .source(STEALTH_JS.to_string())
            .build()
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("stealth params: {e}"))))?;
        page.execute(add_script)
            .await
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("inject stealth: {e}"))))?;

        Ok(Self {
            browser,
            page,
            _handler: handler,
        })
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Navigate and wait until the page reaches "loaded" readyState.
    ///
    /// chromiumoxide's `Page::goto` already drives `Page.navigate` and
    /// awaits the lifecycle event for DOMContentLoaded. A second call
    /// to `wait_for_navigation()` waits for the *next* navigation event,
    /// which never fires on a fully-loaded SPA — that path could hang
    /// until the request_timeout. So we await `goto` only.
    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.page
            .goto(url)
            .await
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("goto {url}: {e}"))))?;
        Ok(())
    }

    /// Save a PNG screenshot to disk. Used by dry-run mode to produce
    /// a `would_submit` artifact — operators can see exactly what state
    /// the form was in when the submit was suppressed.
    pub async fn screenshot(&self, path: &Path) -> Result<()> {
        let png = self
            .page
            .screenshot(ScreenshotParams::builder().full_page(true).build())
            .await
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("screenshot: {e}"))))?;
        tokio::fs::write(path, &png)
            .await
            .map_err(SubmitError::Io)?;
        Ok(())
    }

    /// Set a cookie for the current session. Used to install LinkedIn's
    /// `li_at` session cookie before navigating to gated pages.
    ///
    /// The `secure` parameter is honored, but `SameSite=None` cookies
    /// require `Secure=true` per browser policy — a `None`+`!secure`
    /// cookie is silently rejected by every modern browser, looking
    /// like a navigation bug. We surface a `BadCookie` error so callers
    /// get an actionable message at construction time instead.
    pub async fn set_cookie(
        &self,
        name: &str,
        value: &str,
        domain: &str,
        secure: bool,
    ) -> Result<()> {
        if !secure {
            return Err(SubmitError::Io(std::io::Error::other(
                "set_cookie: SameSite=None requires Secure=true; \
                 callers must pass secure=true for cross-site session cookies",
            )));
        }
        let params = SetCookieParams::builder()
            .name(name.to_string())
            .value(value.to_string())
            .domain(domain.to_string())
            .path("/".to_string())
            .secure(secure)
            .http_only(true)
            .same_site(CookieSameSite::None)
            .build()
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("cookie params: {e}"))))?;
        self.page
            .execute(params)
            .await
            .map_err(|e| SubmitError::Io(std::io::Error::other(format!("set_cookie: {e}"))))?;
        Ok(())
    }

    /// Close the browser. `Browser::close` needs `&mut self`, but
    /// BrowserSession owns the handle by value — take it by value on
    /// close so the handler task gets dropped too.
    pub async fn close(mut self) -> Result<()> {
        let _ = self.browser.close().await;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn stealth_script_sha_is_deterministic() {
        let a = stealth_script_sha256();
        let b = stealth_script_sha256();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // SHA-256 hex
    }

    #[test]
    fn stealth_script_sha_matches_pin() {
        // Pin the SHA against `EXPECTED_STEALTH_SHA`. Any edit to
        // browser/stealth-v2.js must bump that constant in the same
        // commit — silent mitigation drops are caught here, not after
        // an operator runs against real LinkedIn.
        assert_eq!(
            stealth_script_sha256(),
            EXPECTED_STEALTH_SHA,
            "stealth-v2.js changed without bumping EXPECTED_STEALTH_SHA — \
             review the diff and update the constant deliberately"
        );
    }

    #[test]
    fn stealth_script_contains_known_mitigations() {
        // Lock in that the bundled script covers the listed vectors.
        // If a future edit removes one, this test catches it before
        // operators silently lose protection.
        assert!(STEALTH_JS.contains("navigator.webdriver"));
        assert!(STEALTH_JS.contains("chrome.runtime"));
        assert!(STEALTH_JS.contains("permissions.query"));
        assert!(STEALTH_JS.contains("plugins"));
        assert!(STEALTH_JS.contains("languages"));
        assert!(STEALTH_JS.contains("getParameter"));
    }
}
