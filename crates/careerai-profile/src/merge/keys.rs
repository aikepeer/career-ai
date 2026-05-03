use crate::schema::{Education, Experience};

pub(super) fn exp_key(e: &Experience) -> (String, String, String) {
    (
        e.company.to_lowercase(),
        e.title.to_lowercase(),
        e.start.clone(),
    )
}

pub(super) fn exp_loose_key(e: &Experience) -> (String, String) {
    (e.company.to_lowercase(), e.start.clone())
}

pub(crate) fn looks_like_date_garbage(title: &str) -> bool {
    let t = title.trim();
    if t.is_empty() {
        return true;
    }
    date_like_re().is_match(t)
}

fn date_like_re() -> &'static regex::Regex {
    static DATE_LIKE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    DATE_LIKE.get_or_init(|| {
        #[allow(clippy::expect_used)]
        regex::Regex::new(
            r"(?i)^\s*(?:(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\s+)?\d{4}(?:[-/]\d{1,2})?(?:\s*[-–]\s*(?:present|(?:(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\s+)?\d{4}(?:[-/]\d{1,2})?))?\s*$",
        )
        .expect("date_like regex compiles")
    })
}

pub(super) fn edu_key(e: &Education) -> (String, String) {
    (e.institution.to_lowercase(), e.degree.to_lowercase())
}
