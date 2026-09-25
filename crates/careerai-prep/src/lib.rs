//! Interview-preparation generation grounded in a listing and profile.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use careerai_core::config::CoreConfig;
use careerai_db::queries;
use careerai_llm::cache::Cache;
use careerai_llm::trait_def::Llm;
use careerai_llm::types::{LlmRequest, LlmResponse};
use careerai_profile::Profile;

pub mod error;
pub mod research;
pub mod template;
pub mod types;

pub use error::{PrepError, Result};
pub use types::{BulletKeyword, PrepSheet};

/// Generate a prep sheet with the repository's configured backend.
pub async fn generate(root: &Path, cfg: &CoreConfig, application_id: &str) -> Result<PathBuf> {
    let backend = resolve_backend(root, cfg).await?;
    generate_with_llm(root, cfg, application_id, backend.as_ref(), &[]).await
}

/// Generate a prep sheet after fetching explicitly approved employer/news URLs.
pub async fn generate_with_sources(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
    urls: &[String],
    selected_domains: &[String],
) -> Result<PathBuf> {
    let pool = careerai_db::pool_from_path(&careerai_core::paths::database_path(root)).await?;
    let application = queries::find_application_by_id(&pool, application_id).await?;
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;
    let news = research::fetch_approved_sources(urls, &listing.url, selected_domains).await?;
    let backend = resolve_backend(root, cfg).await?;
    generate_with_llm(root, cfg, application_id, backend.as_ref(), &news).await
}

async fn resolve_backend(root: &Path, cfg: &CoreConfig) -> Result<Arc<careerai_llm::Backend>> {
    let cache_root = if cfg.llm.cache_dir.is_empty() {
        careerai_core::paths::cache_dir_for_root(root).join("llm")
    } else {
        let path = PathBuf::from(&cfg.llm.cache_dir);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let cache = Arc::new(Cache::new(cache_root));
    careerai_llm::Backend::resolve(cfg.llm.backend.clone(), &cfg.llm, cache)
        .await
        .map(Arc::new)
        .map_err(|err| PrepError::Backend(err.to_string()))
}

/// Generate and persist one application prep sheet using an injected LLM.
pub async fn generate_with_llm(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
    llm: &(dyn Llm + Send + Sync),
    company_news: &[String],
) -> Result<PathBuf> {
    let db_path = careerai_core::paths::database_path(root);
    let pool = careerai_db::pool_from_path(&db_path).await?;
    let application = queries::find_application_by_id(&pool, application_id).await?;
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;
    let profile_path = careerai_core::paths::profile_path(root);
    let profile_text = std::fs::read_to_string(&profile_path)?;
    let profile =
        Profile::from_yaml(&profile_text).map_err(|err| PrepError::Profile(err.to_string()))?;
    let bullets = bullet_keywords(&profile, &listing.description);
    let request = prep_request(&profile, &listing, &bullets, company_news, cfg);
    let response = llm.complete(&request).await?;
    let sheet = parse_sheet(&response, application_id, &listing, bullets, company_news)?;
    let rendered = template::render_sheet(&sheet)?;
    let output = root.join("prep").join(format!("{application_id}.md"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, rendered)?;
    Ok(output)
}

fn prep_request(
    profile: &Profile,
    listing: &careerai_db::models::Listing,
    bullets: &[BulletKeyword],
    company_news: &[String],
    cfg: &CoreConfig,
) -> LlmRequest {
    let system = "You are an interview coach. Return only valid JSON matching PrepSheet. Ground every claim in the supplied job description, profile bullets, or approved company-news excerpts. Never invent employers, dates, metrics, products, news, or experience.";
    let user = format!(
        "## Job description\nCompany: {}\nTitle: {}\nURL: {}\n{}\n\n## Profile summary\n{}\n\n## Resume bullet map\n{}\n\n## Approved company news\n{}\n\nReturn JSON with fields application_id, job_title, company, jd_summary, likely_topics, behavioral_questions, bullet_to_keyword, company_news, generated_at. Keep the sheet concise.",
        listing.company,
        listing.title,
        listing.url,
        listing.description,
        profile.summary,
        serde_json::to_string(bullets).unwrap_or_default(),
        serde_json::to_string(company_news).unwrap_or_default(),
    );
    LlmRequest {
        system: system.into(),
        profile_block: profile.summary.clone(),
        user,
        prompt_version: format!("{}+prep-sheet", cfg.llm.prompt_version),
        model: cfg.llm.tailor_model.clone(),
        temperature: 0.2,
        max_tokens: 8192,
        cache_profile: true,
    }
}

fn parse_sheet(
    response: &LlmResponse,
    application_id: &str,
    listing: &careerai_db::models::Listing,
    bullets: Vec<BulletKeyword>,
    company_news: &[String],
) -> Result<PrepSheet> {
    let text = response.text.trim();
    let json = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .unwrap_or(text)
        .strip_suffix("```")
        .unwrap_or(text)
        .trim();
    let mut sheet: PrepSheet = serde_json::from_str(json)?;
    sheet.application_id = application_id.into();
    sheet.job_title.clone_from(&listing.title);
    sheet.company.clone_from(&listing.company);
    if sheet.bullet_to_keyword.is_empty() {
        sheet.bullet_to_keyword = bullets;
    }
    if sheet.company_news.is_empty() {
        sheet.company_news = company_news.to_vec();
    }
    Ok(sheet)
}

fn bullet_keywords(profile: &Profile, jd: &str) -> Vec<BulletKeyword> {
    let jd_words = tokenize(jd);
    profile
        .experience
        .iter()
        .flat_map(|experience| experience.bullets.iter())
        .map(|bullet| BulletKeyword {
            bullet: bullet.clone(),
            keywords: tokenize(bullet)
                .into_iter()
                .filter(|word| jd_words.contains(word))
                .take(8)
                .collect(),
        })
        .collect()
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '#')
        .filter(|word| word.len() >= 3)
        .map(str::to_ascii_lowercase)
        .filter(|word| !matches!(word.as_str(), "and" | "the" | "with" | "for" | "from"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_mapping_uses_only_job_description_terms() {
        let mut profile = Profile::default();
        profile
            .experience
            .push(careerai_profile::schema::Experience {
                title: "Engineer".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2020".into(),
                end: "present".into(),
                bullets: vec!["Built Rust services for robotics".into()],
            });
        let mapped = bullet_keywords(&profile, "Rust services and Python");
        assert_eq!(mapped[0].keywords, vec!["rust", "services"]);
        assert!(!mapped[0].keywords.iter().any(|word| word == "robotics"));
    }
}
