//! Salary benchmark lookup (ported from ai-job-search's `salary_lookup.py`).
//!
//! Reads a user-provided `salary_data.json` (union statistics, Glassdoor
//! exports, manually collected benchmarks — any index- or absolute-value
//! dataset) and looks up companies by name with the same tolerant matching
//! as the Python original: legal-suffix stripping (`A/S`, `ApS`, `GmbH`,
//! parentheticals …), Nordic spelling variants (`ø` → `o`, `æ` → `ae` …),
//! substring scoring, and core-word overlap. Optional by design: when no
//! data file exists the salary step is simply skipped.
//!
//! Data contract (identical to the upstream tool):
//! ```json
//! {
//!   "metadata": {
//!     "source": "My Union Statistics 2025",
//!     "index_baseline": 100,
//!     "index_label": "Index",
//!     "baseline_description": "Index 100 = median salary"
//!   },
//!   "companies": [
//!     { "company": "Novo Nordisk A/S", "city": "Bagsværd",
//!       "categories": { "engineering": { "count": 120, "index": 112.3 } } }
//!   ]
//! }
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors surfaced when loading or validating `salary_data.json`.
#[derive(Debug, Error)]
pub enum SalaryError {
    #[error("salary data file not found at {path}; create one or skip the salary step")]
    MissingFile { path: String },
    #[error("invalid salary_data.json: {detail}")]
    Invalid { detail: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SalaryError>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SalaryMetadata {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default = "default_baseline")]
    pub index_baseline: f64,
    #[serde(default = "default_index_label")]
    pub index_label: String,
    #[serde(default)]
    pub baseline_description: Option<String>,
}

fn default_baseline() -> f64 {
    100.0
}

fn default_index_label() -> String {
    "Index".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanySalary {
    pub company: String,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub categories: BTreeMap<String, Category>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Category {
    #[serde(default)]
    pub count: Option<f64>,
    #[serde(default)]
    pub index: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalaryData {
    #[serde(default)]
    pub metadata: SalaryMetadata,
    pub companies: Vec<CompanySalary>,
}

impl SalaryData {
    /// Load + shape-validate `salary_data.json` from `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SalaryError::MissingFile {
                path: path.display().to_string(),
            },
            _ => SalaryError::Io(e),
        })?;
        let data: SalaryData = serde_json::from_str(&raw).map_err(|e| SalaryError::Invalid {
            detail: format!("invalid JSON: {e}"),
        })?;
        // Hard shape checks mirroring the Python validator: companies must
        // be a list of objects with non-empty `company` names.
        for (i, c) in data.companies.iter().enumerate() {
            if c.company.trim().is_empty() {
                return Err(SalaryError::Invalid {
                    detail: format!("companies[{}].company must be a non-empty string", i + 1),
                });
            }
        }
        Ok(data)
    }

    /// Non-fatal usability warnings (duplicate company names), for
    /// `--validate` output.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, c) in self.companies.iter().enumerate() {
            let key = normalize(&c.company);
            if !seen.insert(key) {
                out.push(format!(
                    "Duplicate company name '{}' (companies[{}])",
                    c.company,
                    i + 1
                ));
            }
        }
        out
    }

    /// Best-matching entries for `query`, optionally narrowed to `city`.
    /// Sorted by match score desc, then company name; only entries at or
    /// above the 30-point relevance floor are returned (mirrors the
    /// Python tool).
    pub fn lookup(&self, query: &str, city: Option<&str>) -> Vec<&CompanySalary> {
        let mut scored: Vec<(u8, &CompanySalary)> = self
            .companies
            .iter()
            .filter(|c| city_matches(c, city))
            .filter_map(|c| {
                let score = match_score(query, &c.company)?;
                (score >= MIN_MATCH_SCORE).then_some((score, c))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.company.cmp(&b.1.company)));
        scored.into_iter().map(|(_, c)| c).collect()
    }
}

const MIN_MATCH_SCORE: u8 = 30;

/// Case-insensitive city containment — plain or anglicized (so "København"
/// matches a query for "Kobenhavn").
fn city_matches(entry: &CompanySalary, city: Option<&str>) -> bool {
    let Some(city) = city else { return true };
    let city = city.to_lowercase();
    let entry_city = entry.city.clone().unwrap_or_default().to_lowercase();
    city.is_empty()
        || entry_city.is_empty()
        || entry_city.contains(&city)
        || anglicize(&entry_city).contains(&anglicize(&city))
}

/// Score a query against one company name, `Some(0..=100)` when there is
/// any signal, `None` on a hard miss. Port of the Python tool's
/// `match_score_optimized` ladder: exact → substring → anglicized-equal →
/// anglicized-substring → core-word overlap.
fn match_score(query: &str, entry_name: &str) -> Option<u8> {
    let q_norm = normalize(query);
    let n_norm = normalize(entry_name);
    if q_norm.is_empty() || n_norm.is_empty() {
        return None;
    }
    if q_norm == n_norm {
        return Some(100);
    }
    if q_norm.contains(&n_norm) {
        // Integer-math port of `int(ratio * 10)`: floor of (short*10/long).
        let ratio10 = n_norm.len().saturating_mul(10) / q_norm.len().max(1);
        if n_norm.len() <= 4 && ratio10 < 5 {
            return word_overlap_fallback(query, entry_name, 80);
        }
        return Some(80 + u8::try_from(ratio10).unwrap_or(10));
    }
    if n_norm.contains(&q_norm) {
        let ratio10 = q_norm.len().saturating_mul(10) / n_norm.len().max(1);
        if q_norm.len() > 4 || ratio10 >= 5 {
            return Some(80 + u8::try_from(ratio10).unwrap_or(10));
        }
    }
    let q_ang = anglicize(&q_norm);
    let n_ang = anglicize(&n_norm);
    if q_ang == n_ang {
        return Some(85);
    }
    if q_ang.contains(&n_ang) || n_ang.contains(&q_ang) {
        let shorter = q_ang.len().min(n_ang.len());
        let longer = q_ang.len().max(n_ang.len());
        if shorter <= 4 && shorter.saturating_mul(2) < longer {
            return word_overlap_fallback(query, entry_name, 75);
        }
        return Some(75);
    }
    core_word_score(query, entry_name)
}

/// When a substring hit is too short to trust, fall back to core-word
/// overlap and return `base` only if a word intersects.
fn word_overlap_fallback(query: &str, entry_name: &str, base: u8) -> Option<u8> {
    let q_words = core_words(query);
    let n_words = core_words(entry_name);
    if q_words.is_empty() || n_words.is_empty() {
        return None;
    }
    let q_ang: std::collections::HashSet<String> = q_words.iter().map(|w| anglicize(w)).collect();
    let n_ang: std::collections::HashSet<String> = n_words.iter().map(|w| anglicize(w)).collect();
    if q_words.iter().any(|w| n_words.contains(w)) || q_ang.intersection(&n_ang).next().is_some() {
        Some(base)
    } else {
        None
    }
}

/// Core-word overlap scoring: 70 for a single-word hit, else
/// `30 + coverage * 40` where coverage = shared fraction of query words.
fn core_word_score(query: &str, entry_name: &str) -> Option<u8> {
    let q_words = core_words(query);
    let n_words = core_words(entry_name);
    if q_words.is_empty() || n_words.is_empty() {
        return None;
    }
    let n_ang: std::collections::HashSet<String> = n_words.iter().map(|w| anglicize(w)).collect();
    let overlap: Vec<&String> = q_words
        .iter()
        .filter(|w| n_words.contains(w) || n_ang.contains(&anglicize(w)))
        .collect();
    if overlap.is_empty() {
        return None;
    }
    if q_words.len() == 1 {
        return Some(70);
    }
    let coverage40 = overlap.len().saturating_mul(40) / q_words.len().max(1);
    Some(30 + u8::try_from(coverage40).unwrap_or(40))
}

/// Lowercase, strip legal suffixes + parentheticals, drop everything
/// non-alphanumeric. Same patterns as the Python tool.
fn normalize(s: &str) -> String {
    let mut out = s.to_lowercase();
    for pat in strip_patterns() {
        out = pat.replace_all(&out, "").into_owned();
    }
    out.retain(char::is_alphanumeric);
    out
}

/// Nordic character variants to anglicized equivalents.
fn anglicize(s: &str) -> String {
    let mut out = s.to_string();
    for (danish, english) in SPELLING_VARIANTS {
        out = out.replace(danish, english);
    }
    out
}

/// Meaningful words of a company name (length > 1) after stripping noise.
fn core_words(s: &str) -> Vec<String> {
    let mut out = s.to_lowercase();
    for pat in strip_patterns() {
        out = pat.replace_all(&out, "").into_owned();
    }
    out.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_string)
        .collect()
}

const SPELLING_VARIANTS: &[(&str, &str)] = &[
    ("ø", "o"),
    ("æ", "ae"),
    ("å", "aa"),
    ("ö", "o"),
    ("ä", "ae"),
    ("ü", "u"),
];

/// Legal suffixes and noise stripped when matching company names.
fn strip_patterns() -> &'static [Regex] {
    static PATTERNS: std::sync::OnceLock<Vec<Regex>> = std::sync::OnceLock::new();
    #[allow(clippy::expect_used)] // literal regexes — cannot fail
    PATTERNS.get_or_init(|| {
        [
            r"\ba/s\b",
            r"\baps\b",
            r"\bi/s\b",
            r"\bp/s\b",
            r"\bk/s\b",
            r"\bivs\b",
            r"\bamba\b",
            r"\ba\.m\.b\.a\.?\b",
            r"\(vg\)",
            r"\(.*?\)",
            r"\bdanmark\b",
            r"\bdenmark\b",
            r"\bscandinavia\b",
            r"\bnordic\b",
            r"\bgroup\b",
            r"\bholding\b",
            r",\s*.*$",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("literal strip pattern"))
        .collect()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_data() -> SalaryData {
        serde_json::from_value(serde_json::json!({
            "metadata": {
                "source": "Union Stats 2025",
                "index_baseline": 100,
                "index_label": "Index"
            },
            "companies": [
                {"company": "Novo Nordisk A/S", "city": "Bagsværd",
                 "categories": {"engineering": {"count": 120, "index": 112.3}}},
                {"company": "Ørsted", "city": "Fredericia",
                 "categories": {"all_employees": {"count": 500, "index": 98.5}}},
                {"company": "Acme Robotics GmbH", "city": "Berlin",
                 "categories": {"engineering": {"count": 40, "index": 105.0}}},
                {"company": "Beta Corp", "city": null,
                 "categories": {"all_employees": {"index": 91.0}}}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn exact_match_scores_hundred() {
        let d = sample_data();
        let hits = d.lookup("Novo Nordisk A/S", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].company, "Novo Nordisk A/S");
    }

    #[test]
    fn strips_legal_suffixes_and_parentheticals() {
        let d = sample_data();
        // Query without the suffix must still match; parenthetical
        // "(Denmark)" in a query is ignored.
        let hits = d.lookup("Novo Nordisk", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].company, "Novo Nordisk A/S");
        let hits2 = d.lookup("Acme Robotics (Germany)", None);
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].company, "Acme Robotics GmbH");
    }

    #[test]
    fn matches_anglicized_spelling_variants() {
        let d = sample_data();
        // Query "Orsted" (no ø) must match the entry "Ørsted".
        let hits = d.lookup("Orsted", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].company, "Ørsted");
        // And the reverse: query with the native character.
        let hits2 = d.lookup("Ørsted", None);
        assert_eq!(hits2.len(), 1);
    }

    #[test]
    fn city_narrows_results() {
        let d = sample_data();
        let hits = d.lookup("Nordisk", Some("Bagsværd"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].company, "Novo Nordisk A/S");
        // City mismatch → nothing.
        let hits2 = d.lookup("Nordisk", Some("Mumbai"));
        assert!(hits2.is_empty());
        // Anglicized city query still matches.
        let hits3 = d.lookup("Nordisk", Some("Bagsvaerd"));
        assert_eq!(hits3.len(), 1);
    }

    #[test]
    fn partial_core_word_match_above_floor() {
        let d = sample_data();
        // "Acme" alone → single-word hit.
        let hits = d.lookup("Acme", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].company, "Acme Robotics GmbH");
    }

    #[test]
    fn unrelated_query_returns_nothing() {
        let d = sample_data();
        assert!(d.lookup("Starbucks", None).is_empty());
        assert!(d.lookup("", None).is_empty());
    }

    #[test]
    fn results_sorted_by_score_then_name() {
        let d = sample_data();
        // "Corp" is a substring of "Beta Corp" (exact-normalized hit) and
        // appears in no other entry — single hit, name is Beta Corp.
        let hits = d.lookup("Corp", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].company, "Beta Corp");
    }

    #[test]
    fn categories_expose_count_and_index() {
        let d = sample_data();
        let hits = d.lookup("Novo Nordisk", None);
        let eng = hits[0].categories.get("engineering").unwrap();
        assert_eq!(eng.count, Some(120.0));
        assert_eq!(eng.index, Some(112.3));
    }

    #[test]
    fn load_missing_file_is_helpful_error() {
        let err = SalaryData::load(Path::new("/nonexistent/salary_data.json")).unwrap_err();
        assert!(
            matches!(err, SalaryError::MissingFile { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn load_rejects_invalid_json_and_empty_company() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("salary_data.json");
        std::fs::write(&p, "{not json").unwrap();
        assert!(matches!(
            SalaryData::load(&p).unwrap_err(),
            SalaryError::Invalid { .. }
        ));

        std::fs::write(&p, r#"{"companies": [{"company": "  "}]}"#).unwrap();
        assert!(matches!(
            SalaryData::load(&p).unwrap_err(),
            SalaryError::Invalid { .. }
        ));
    }

    #[test]
    fn warnings_flag_duplicate_company_names() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("salary_data.json");
        std::fs::write(
            &p,
            r#"{"companies": [
                {"company": "Acme A/S"},
                {"company": "Acme"}
            ]}"#,
        )
        .unwrap();
        let d = SalaryData::load(&p).unwrap();
        assert_eq!(d.warnings().len(), 1);
        assert!(d.warnings()[0].contains("Acme"));
    }

    #[test]
    fn roundtrip_via_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("salary_data.json");
        std::fs::write(&p, serde_json::to_string_pretty(&sample_data()).unwrap()).unwrap();
        let d = SalaryData::load(&p).unwrap();
        assert!((d.metadata.index_baseline - 100.0).abs() < f64::EPSILON);
        assert_eq!(d.metadata.index_label, "Index");
        assert_eq!(d.companies.len(), 4);
    }
}
