//! `careerai prep` — generate a persisted interview study sheet.

use std::path::Path;

use anyhow::Result;
use careerai_core::config::CoreConfig;

pub async fn run(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
    news_urls: &[String],
    news_domains: &[String],
) -> Result<()> {
    let path = if news_urls.is_empty() {
        careerai_prep::generate(root, cfg, application_id).await?
    } else {
        careerai_prep::generate_with_sources(root, cfg, application_id, news_urls, news_domains)
            .await?
    };
    println!("prep sheet: {}", path.display());
    Ok(())
}
