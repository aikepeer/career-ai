//! Tera templating for resume + cover letter Markdown.
//!
//! Autoescape is deliberately OFF — the output is Markdown, not HTML,
//! and bullets may contain Markdown-sensitive characters that we
//! neutralize via the `escape_md` filter instead.

use std::collections::HashMap;
use std::sync::OnceLock;

use careerai_tailor::model::{CoverLetter, ResumeView};
use tera::{Context, Tera, Value};

use crate::error::{RenderError, Result};

pub(crate) const RESUME_TEMPLATE: &str = include_str!("../../../templates/resume.md.tera");
pub(crate) const COVER_LETTER_TEMPLATE: &str =
    include_str!("../../../templates/cover_letter.md.tera");

const RESUME_NAME: &str = "resume.md.tera";
const COVER_LETTER_NAME: &str = "cover_letter.md.tera";

/// Escape characters that have special meaning in Markdown tables, code,
/// emphasis, or Tera delimiters. The output is still valid Markdown text.
fn escape_md_filter(
    value: &Value,
    _args: &HashMap<String, Value>,
) -> std::result::Result<Value, tera::Error> {
    let s = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '|' => out.push_str("\\|"),
            '`' => out.push_str("\\`"),
            '_' => out.push_str("\\_"),
            '*' => out.push_str("\\*"),
            '{' => match chars.peek() {
                Some('{') => {
                    chars.next();
                    out.push_str("&#123;&#123;");
                }
                Some('%') => {
                    chars.next();
                    out.push_str("&#123;%");
                }
                _ => out.push('{'),
            },
            other => out.push(other),
        }
    }
    tera::to_value(out).map_err(tera::Error::from)
}

/// Join a list of strings with newlines. Useful for turning a
/// `Vec<String>` into a single block in a template.
fn joinlines_filter(
    value: &Value,
    _args: &HashMap<String, Value>,
) -> std::result::Result<Value, tera::Error> {
    let items: Vec<String> = match value {
        Value::Array(arr) => arr
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        Value::String(s) => vec![s.clone()],
        other => vec![other.to_string()],
    };
    tera::to_value(items.join("\n")).map_err(tera::Error::from)
}

fn build_engine() -> Result<Tera> {
    let mut tera = Tera::default();
    tera.autoescape_on(vec![]);
    tera.register_filter("escape_md", escape_md_filter);
    tera.register_filter("joinlines", joinlines_filter);
    tera.add_raw_template(RESUME_NAME, RESUME_TEMPLATE)?;
    tera.add_raw_template(COVER_LETTER_NAME, COVER_LETTER_TEMPLATE)?;
    Ok(tera)
}

/// Process-wide Tera instance. Parsing both templates + registering
/// filters costs ~30ms on first use; cache it so subsequent renders
/// pay only the render cost (single-digit ms).
fn engine() -> Result<&'static Tera> {
    static ENGINE: OnceLock<std::result::Result<Tera, String>> = OnceLock::new();
    match ENGINE.get_or_init(|| build_engine().map_err(|e| e.to_string())) {
        Ok(t) => Ok(t),
        Err(msg) => Err(RenderError::Io(std::io::Error::other(format!(
            "tera engine init failed: {msg}"
        )))),
    }
}

pub fn render_resume(view: &ResumeView, personal_name: &str) -> Result<String> {
    let tera = engine()?;
    let mut ctx = {
        let json = serde_json::to_value(view)
            .map_err(|e| tera::Error::msg(format!("serialize ResumeView: {e}")))?;
        Context::from_value(json)?
    };
    ctx.insert("personal_name", personal_name);
    Ok(tera.render(RESUME_NAME, &ctx)?)
}

pub fn render_cover_letter(
    letter: &CoverLetter,
    personal_name: &str,
    listing_company: &str,
    date: &str,
) -> Result<String> {
    let tera = engine()?;
    let mut ctx = Context::new();
    ctx.insert("body", &letter.body);
    ctx.insert("personal_name", personal_name);
    ctx.insert("listing_company", listing_company);
    ctx.insert("date", date);
    Ok(tera.render(COVER_LETTER_NAME, &ctx)?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Education, Links, Personal, Skills};
    use careerai_tailor::model::{ExperienceView, ProjectView};

    fn fixture_view() -> ResumeView {
        ResumeView {
            personal: Personal {
                name: "Jane Doe".into(),
                email: "jane@example.com".into(),
                phone: "+1-555-0100".into(),
                location: "Remote".into(),
                links: Links {
                    github: "https://github.com/jane".into(),
                    linkedin: "https://linkedin.com/in/jane".into(),
                    portfolio: String::new(),
                },
            },
            summary: "Backend engineer".into(),
            skills: Skills {
                languages: vec!["Rust".into(), "Python".into()],
                frameworks: vec!["Tokio".into()],
                tools: vec!["Docker".into()],
            },
            experience: vec![ExperienceView {
                title: "Senior Engineer".into(),
                company: "Acme".into(),
                location: Some("Remote".into()),
                start: "2022-01".into(),
                end: "present".into(),
                bullets: vec!["shipped | pipelines".into(), "owned _core_ infra".into()],
            }],
            education: vec![Education {
                degree: "BSc CS".into(),
                institution: "State U".into(),
                start: "2015".into(),
                end: "2019".into(),
            }],
            projects: vec![ProjectView {
                name: "careerai".into(),
                url: Some("https://example.com".into()),
                bullets: vec!["rust daemon".into()],
            }],
        }
    }

    #[test]
    fn render_resume_contains_name_and_titles() {
        let view = fixture_view();
        let md = render_resume(&view, &view.personal.name).unwrap();
        assert!(md.contains("# Jane Doe"), "name header missing: {md}");
        assert!(md.contains("Senior Engineer"), "title missing");
        assert!(md.contains("Acme"), "company missing");
        assert!(md.contains("BSc CS"), "education missing");
    }

    #[test]
    fn render_resume_escapes_pipe_in_bullet() {
        let view = fixture_view();
        let md = render_resume(&view, &view.personal.name).unwrap();
        // Pipe in "shipped | pipelines" should become "shipped \| pipelines".
        assert!(
            md.contains("shipped \\| pipelines"),
            "pipe not escaped in: {md}"
        );
    }

    #[test]
    fn render_resume_escapes_underscores() {
        let view = fixture_view();
        let md = render_resume(&view, &view.personal.name).unwrap();
        assert!(
            md.contains("owned \\_core\\_ infra"),
            "underscores not escaped in: {md}"
        );
    }

    #[test]
    fn render_cover_letter_contains_date_and_company() {
        let letter = CoverLetter {
            body: "I am excited to apply.".into(),
        };
        let md = render_cover_letter(&letter, "Jane Doe", "Acme Corp", "2026-04-24").unwrap();
        assert!(md.contains("2026-04-24"));
        assert!(md.contains("Acme Corp"));
        assert!(md.contains("Jane Doe"));
        assert!(md.contains("I am excited to apply."));
    }

    #[test]
    fn escape_md_neutralizes_tera_delimiters() {
        let args = HashMap::new();
        let v = tera::to_value("look: {{ foo }} and {% raw %}").unwrap();
        let got = escape_md_filter(&v, &args).unwrap();
        let s = got.as_str().unwrap();
        assert!(s.contains("&#123;&#123; foo }}"), "got: {s}");
        assert!(s.contains("&#123;% raw %}"), "got: {s}");
    }

    #[test]
    fn joinlines_joins_array_with_newlines() {
        let args = HashMap::new();
        let v = tera::to_value(vec!["a", "b", "c"]).unwrap();
        let got = joinlines_filter(&v, &args).unwrap();
        assert_eq!(got.as_str().unwrap(), "a\nb\nc");
    }
}
