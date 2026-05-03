use std::time::Duration;

use careerai_submit::browser_session::{BrowserSession, BrowserSessionConfig};
use careerai_submit::credentials;
use careerai_submit::Credential;
use rand::Rng;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::base::{RawListing, SourceError};
use crate::linkedin_browser_parser::{build_search_url, parse_search_html};

use super::source::LinkedinBrowserSource;

const COOKIE_DOMAIN: &str = ".linkedin.com";
const COOKIE_NAME: &str = "li_at";

const PAGE_JITTER_MS_MIN: u64 = 1_500;
const PAGE_JITTER_MS_MAX: u64 = 3_500;

pub(super) async fn run_discover(
    src: &LinkedinBrowserSource,
) -> Result<Vec<RawListing>, SourceError> {
    let li_at = credentials::load(&Credential::for_source("linkedin", COOKIE_NAME))
        .map_err(|e| SourceError::Parse(format!("linkedin-browser cookie load failed: {e}")))?;

    let session_cfg = BrowserSessionConfig {
        headless: src.cfg.headless,
        request_timeout_seconds: src.cfg.action_timeout_seconds,
        ..BrowserSessionConfig::default()
    };

    let session = BrowserSession::launch(&session_cfg).await.map_err(|e| {
        SourceError::Parse(format!(
            "linkedin-browser launch failed (is Chromium installed?): {e}"
        ))
    })?;

    let inner = scrape_pages(src, &session, &li_at).await;
    if let Err(e) = session.close().await {
        warn!(
            target: "linkedin-browser",
            error = %e,
            "browser close failed (continuing with scraped results)"
        );
    }
    inner
}

async fn scrape_pages(
    src: &LinkedinBrowserSource,
    session: &BrowserSession,
    li_at: &str,
) -> Result<Vec<RawListing>, SourceError> {
    session
        .set_cookie(COOKIE_NAME, li_at, COOKIE_DOMAIN)
        .await
        .map_err(|e| SourceError::Parse(format!("linkedin-browser cookie install failed: {e}")))?;

    let pages = src.cfg.max_pages.max(1);
    let mut all: Vec<RawListing> = Vec::new();

    for page_idx in 0..pages {
        if let Some(rl) = &src.rate_limiter {
            rl.until_ready().await;
        }

        let url = build_search_url(&src.cfg, page_idx);
        debug!(
            target: "linkedin-browser",
            page = page_idx + 1,
            url = %url,
            "navigating to page"
        );
        session.navigate(&url).await.map_err(|e| {
            SourceError::Parse(format!(
                "linkedin-browser nav page={} failed: {e}",
                page_idx + 1
            ))
        })?;

        let html = capture_outer_html(session).await?;
        let parsed = parse_search_html(&html);
        info!(
            target: "linkedin-browser",
            page = page_idx + 1,
            cards = parsed.len(),
            "scraped page"
        );
        all.extend(parsed);

        if page_idx + 1 < pages {
            let jitter_ms = jitter_ms();
            sleep(Duration::from_millis(jitter_ms)).await;
        }
    }

    Ok(all)
}

fn jitter_ms() -> u64 {
    let mut rng = rand::rng();
    rng.random_range(PAGE_JITTER_MS_MIN..=PAGE_JITTER_MS_MAX)
}

async fn capture_outer_html(session: &BrowserSession) -> Result<String, SourceError> {
    session
        .page()
        .content()
        .await
        .map_err(|e| SourceError::Parse(format!("linkedin-browser page content fetch failed: {e}")))
}
