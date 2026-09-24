//! Markdown rendering for interview-preparation sheets.

use tera::{Context, Tera};

use crate::error::{PrepError, Result};
use crate::types::PrepSheet;

const TEMPLATE: &str = include_str!("../templates/prep_sheet.md.tera");

pub fn render_sheet(sheet: &PrepSheet) -> Result<String> {
    let context =
        Context::from_serialize(sheet).map_err(|err| PrepError::Template(err.to_string()))?;
    Tera::one_off(TEMPLATE, &context, true).map_err(|err| PrepError::Template(err.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::types::BulletKeyword;
    use chrono::TimeZone;

    #[test]
    fn render_sheet_contains_grounded_sections() {
        let sheet = PrepSheet {
            application_id: "app-1".into(),
            job_title: "ML Engineer".into(),
            company: "Acme".into(),
            jd_summary: "Build models".into(),
            likely_topics: vec!["Rust".into()],
            behavioral_questions: vec!["Tell me about a system".into()],
            bullet_to_keyword: vec![BulletKeyword {
                bullet: "Built a pipeline".into(),
                keywords: vec!["pipeline".into()],
            }],
            company_news: vec![],
            generated_at: chrono::Utc.timestamp_opt(0, 0).unwrap(),
        };
        let rendered = render_sheet(&sheet).unwrap();
        assert!(rendered.contains("# Interview prep — ML Engineer @ Acme"));
        assert!(rendered.contains("**Built a pipeline**"));
        assert!(rendered.contains("## Company recent news"));
    }
}
