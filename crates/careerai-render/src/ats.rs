//! ATS text-layer verification for rendered resumes (ported from
//! ai-job-search's `/apply` Step 5d).
//!
//! An ATS parses the PDF's embedded **text layer**, not the rendered
//! page. A resume that looks fine visually can still extract as garbage
//! (icon glyphs where the email should be, `(cid:NNN)` glyph-name
//! markers, scrambled reading order). This module verifies what a parser
//! actually sees:
//!
//! 1. **Parseability** — text extracts at all, with no `(cid:NNN)` or
//!    U+FFFD garbage runs.
//! 2. **Contact as literal text** — email and phone must survive as
//!    printable text; a contact carried only by an icon or a hyperlink
//!    target is invisible to an ATS.
//! 3. **Keyword coverage** — the JD's top terms vs. what the extraction
//!    contains. Honest reporting only: a keyword the profile genuinely
//!    lacks stays missing; we never stuff keywords here.
//!
//! The check is advisory: it reports and logs, it never blocks a render.
//! Rendering here uses a single-column template, so the reading-order
//! class of failure (multi-column interleave) cannot occur; the checks
//! below cover the failures that can.

use std::path::Path;

use regex::Regex;
use tracing::{info, warn};

use crate::error::{RenderError, Result};

/// One ATS check result for the resume PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtsReport {
    /// Text extractor used ("pdf-extract").
    pub extractor: &'static str,
    /// Characters of extracted text.
    pub text_len: usize,
    /// Garbage-run markers found in the extraction.
    pub garbage: Vec<GarbageKind>,
    /// Per-contact-field presence.
    pub contact: Vec<ContactCheck>,
    /// Per-keyword presence in the extracted text.
    pub keywords: Vec<KeywordHit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GarbageKind {
    /// `(cid:NNN)` — a glyph-name escape from an icon font.
    CidRef,
    /// U+FFFD — a byte that failed to decode; non-ASCII text is broken.
    ReplacementChar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactCheck {
    pub field: &'static str,
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordHit {
    pub keyword: String,
    pub present: bool,
}

/// Extract the text layer of a PDF. Uses `pdf-extract` (pure Rust,
/// embedded in the binary — no poppler/pdftotext dependency).
pub fn extract_pdf_text(path: &Path) -> Result<String> {
    pdf_extract::extract_text(path).map_err(|e| RenderError::Ats {
        stage: "extract",
        detail: e.to_string(),
    })
}

/// Verify a resume PDF's text layer against contact details + JD keywords.
pub fn verify_pdf(path: &Path, email: &str, phone: &str, keywords: &[String]) -> Result<AtsReport> {
    let text = extract_pdf_text(path)?;
    Ok(verify_text(&text, email, phone, keywords))
}

/// Pure-text verification core (unit-testable without a PDF).
pub fn verify_text(text: &str, email: &str, phone: &str, keywords: &[String]) -> AtsReport {
    let mut garbage = Vec::new();
    let cid_count = count_cid_refs(text);
    if cid_count > 0 {
        garbage.push(GarbageKind::CidRef);
    }
    if text.contains('\u{FFFD}') {
        garbage.push(GarbageKind::ReplacementChar);
    }

    let email_ok = !email.is_empty() && text.to_lowercase().contains(&email.to_lowercase());
    let phone_ok = phone_present(text, phone);

    let mut kw = Vec::with_capacity(keywords.len());
    let lower_text = text.to_lowercase();
    // Stemmed token set for inflection-tolerant matching: a resume that
    // says "building" covers a JD that says "build". Only consulted when
    // the verbatim match misses.
    let stems: std::collections::HashSet<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|t| {
            let s = stem(t).to_lowercase();
            (s.len() >= 4).then_some(s)
        })
        .collect();
    for k in keywords {
        let present =
            !k.is_empty() && (lower_text.contains(&k.to_lowercase()) || stems.contains(stem(k)));
        kw.push(KeywordHit {
            keyword: k.clone(),
            present,
        });
    }

    AtsReport {
        extractor: "pdf-extract",
        text_len: text.len(),
        garbage,
        contact: vec![
            ContactCheck {
                field: "email",
                present: email_ok,
            },
            ContactCheck {
                field: "phone",
                present: phone_ok,
            },
        ],
        keywords: kw,
    }
}

/// Phone check: compare digit-only normalizations so formatting
/// differences (spaces, dashes, parens) never cause a false miss.
fn phone_present(text: &str, phone: &str) -> bool {
    if phone.is_empty() {
        return false;
    }
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 7 {
        // Too short to be a real phone — fall back to verbatim match.
        return text.contains(phone);
    }
    let text_digits: String = text.chars().filter(char::is_ascii_digit).collect();
    text_digits.contains(&digits)
}

/// Crude suffix-stripping stem used ONLY for coverage matching (both
/// sides get the same treatment, so consistency is what matters, not
/// linguistic correctness). Strips one common inflection suffix; the
/// caller guards against over-short stems and lowercases the result.
fn stem(t: &str) -> &str {
    let t = t
        .strip_suffix("ing")
        .or_else(|| t.strip_suffix("ed"))
        .unwrap_or(t);
    t.strip_suffix("es")
        .or_else(|| t.strip_suffix("s"))
        .unwrap_or(t)
}

fn count_cid_refs(text: &str) -> usize {
    static CID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    #[allow(clippy::expect_used)] // literal regex — cannot fail
    let re = CID_RE.get_or_init(|| Regex::new(r"\(cid:\d+\)").expect("cid regex is valid"));
    re.find_iter(text).count()
}

/// Derive the JD keyword list for coverage checking. Normal words:
/// lowercase tokens of length >= 4, stopwords removed. Acronyms (e.g.
/// `RAG`, `LLM`, `ROS2`) are kept from length 2 because they are
/// high-signal in this niche and too short for the length filter.
/// Ranked by frequency (ties: alphabetical), capped at `limit`.
/// Deterministic and cheap — no LLM involved.
pub fn extract_jd_keywords(jd: &str, limit: usize) -> Vec<String> {
    use std::collections::HashMap;
    let mut freq: HashMap<String, usize> = HashMap::new();
    for raw in jd.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_lowercase();
        let is_acronym = raw.len() >= 2
            && raw.chars().any(char::is_alphabetic)
            && raw.chars().all(|c| c.is_uppercase() || c.is_ascii_digit());
        let in_stopwords = STOPWORDS.contains(&lower.as_str());
        let keep = if is_acronym {
            !in_stopwords
        } else {
            lower.len() >= 4 && !in_stopwords
        };
        if keep {
            *freq.entry(lower).or_insert(0) += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = freq.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(limit);
    ranked.into_iter().map(|(k, _)| k).collect()
}

/// Log an ATS report the way the pipeline consumes it.
pub fn log_report(report: &AtsReport) {
    let covered = report.keywords.iter().filter(|k| k.present).count();
    let missing: Vec<&str> = report
        .keywords
        .iter()
        .filter(|k| !k.present)
        .map(|k| k.keyword.as_str())
        .collect();
    info!(
        target = "render",
        extractor = report.extractor,
        text_len = report.text_len,
        covered_keywords = covered,
        total_keywords = report.keywords.len(),
        garbage = ?report.garbage,
        contact = ?report.contact,
        "ATS text-layer check complete"
    );
    if !report.garbage.is_empty() {
        warn!(
            target = "render",
            "ATS: garbage markers found in resume PDF text layer"
        );
    }
    if let Some(c) = report.contact.iter().find(|c| !c.present) {
        warn!(
            target = "render",
            field = c.field,
            "ATS: contact detail missing from PDF text layer"
        );
    }
    if !missing.is_empty() {
        info!(
            target = "render",
            missing = ?missing,
            "ATS: JD keywords absent from resume text layer (honest gaps — not stuffed)"
        );
    }
}

/// Common English function words dropped from JD keyword extraction.
/// Not exhaustive — long-tail noise is handled by the frequency ranking.
const STOPWORDS: &[&str] = &[
    "about",
    "above",
    "after",
    "again",
    "against",
    "also",
    "among",
    "another",
    "around",
    "because",
    "before",
    "being",
    "below",
    "between",
    "could",
    "during",
    "each",
    "either",
    "every",
    "first",
    "from",
    "further",
    "having",
    "here",
    "however",
    "into",
    "least",
    "less",
    "like",
    "more",
    "most",
    "much",
    "must",
    "never",
    "other",
    "over",
    "same",
    "should",
    "since",
    "some",
    "still",
    "such",
    "than",
    "that",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "under",
    "until",
    "very",
    "well",
    "what",
    "when",
    "where",
    "which",
    "while",
    "will",
    "with",
    "within",
    "without",
    "would",
    "your",
    "yours",
    "work",
    "role",
    "team",
    "will",
    "candidate",
    "company",
    "please",
    "able",
    "build",
    "working",
    "experience",
    "years",
    "year",
    "skills",
    "including",
    "using",
    "related",
    "looking",
    "join",
    "great",
    "good",
    "strong",
    "knowledge",
    "ability",
    "understanding",
    "required",
    "preferred",
    "qualifications",
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_text() -> String {
        "Alice Kumar\nSenior ML Engineer\n\
         alice.kumar@example.com\n+91 98765 43210\n\
         Built RAG pipelines for LLM applications at Acme Robotics.\n\
         Rust, Python, Kubernetes, ROS2."
            .to_string()
    }

    #[test]
    fn detects_cid_refs_and_replacement_chars() {
        let text = "Email: \u{FFFD}\u{FFFD} (cid:12) (cid:97) ok text";
        let r = verify_text(text, "a@b.co", "+91 98765 43210", &[]);
        assert!(r.garbage.contains(&GarbageKind::CidRef));
        assert!(r.garbage.contains(&GarbageKind::ReplacementChar));

        let clean = verify_text("plain text here", "a@b.co", "", &[]);
        assert!(clean.garbage.is_empty());
    }

    #[test]
    fn email_must_appear_as_literal_text_case_insensitive() {
        let r = verify_text(&sample_text(), "alice.kumar@example.com", "", &[]);
        assert!(r.contact[0].present);
        // Same email, different case — still found.
        let r2 = verify_text(
            &sample_text().to_uppercase(),
            "alice.kumar@example.com",
            "",
            &[],
        );
        assert!(r2.contact[0].present);
        // Missing entirely.
        let r3 = verify_text("no contact info here", "bob@other.io", "", &[]);
        assert!(!r3.contact[0].present);
    }

    #[test]
    fn phone_matches_digit_normalized() {
        // Text formats the phone with parens/spaces; profile stores dashes.
        let text = "Phone: +91 (98765) 43210";
        let r = verify_text(text, "", "+91-98765-43210", &[]);
        assert!(r.contact[1].present);
        // Different number is a miss.
        let r2 = verify_text(text, "", "+1-555-0100", &[]);
        assert!(!r2.contact[1].present);
    }

    #[test]
    fn keyword_coverage_reports_present_and_missing() {
        let r = verify_text(
            &sample_text(),
            "",
            "",
            &["rag".into(), "kubernetes".into(), "waymo".into()],
        );
        let find = |k: &str| r.keywords.iter().find(|h| h.keyword == k).unwrap();
        assert!(find("rag").present);
        assert!(find("kubernetes").present);
        assert!(!find("waymo").present);
    }

    #[test]
    fn empty_keywords_are_never_reported_present() {
        let r = verify_text(&sample_text(), "", "", &[String::new()]);
        assert!(!r.keywords[0].present);
    }

    #[test]
    fn jd_keywords_drop_stopwords_and_rank_by_frequency() {
        let jd = "We are looking for a senior Rust engineer. Rust and Rust again. \
                  The engineer will build robust pipelines with Kubernetes.";
        let kw = extract_jd_keywords(jd, 10);
        assert_eq!(kw[0], "rust");
        assert!(kw.contains(&"engineer".to_string()));
        assert!(kw.contains(&"kubernetes".to_string()));
        assert!(kw.contains(&"pipelines".to_string()));
        assert!(!kw.contains(&"with".to_string()));
        assert!(!kw.contains(&"are".to_string()));
        assert!(!kw.contains(&"we".to_string()));
        assert!(!kw.contains(&"for".to_string()));
        assert!(kw.len() <= 10);
    }

    #[test]
    fn jd_keywords_respect_limit() {
        let jd = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi";
        let kw = extract_jd_keywords(jd, 5);
        assert_eq!(kw.len(), 5);
    }

    #[test]
    fn jd_keywords_keep_acronyms_from_length_2() {
        let jd = "RAG and LLM pipelines; also ML systems and ROS2 firmware.";
        let kw = extract_jd_keywords(jd, 10);
        for acro in ["rag", "llm", "ml", "ros2"] {
            assert!(kw.contains(&acro.to_string()), "{acro} missing from {kw:?}");
        }
        assert!(!kw.contains(&"and".to_string()));
    }

    #[test]
    fn keyword_match_tolerates_inflections() {
        // Resume says "building" — JD keyword "build" must count as covered.
        let r = verify_text("Building RAG systems daily", "", "", &["build".into()]);
        assert!(r.keywords[0].present);
        // "pipelines" covers "pipeline".
        let r2 = verify_text("Ship pipelines", "", "", &["pipeline".into()]);
        assert!(r2.keywords[0].present);
        // Unrelated word still missing.
        let r3 = verify_text("Building RAG systems", "", "", &["waymo".into()]);
        assert!(!r3.keywords[0].present);
    }

    #[test]
    fn jd_keywords_ignore_short_tokens() {
        let kw = extract_jd_keywords("go rust c++ a b c io", 10);
        assert!(!kw.iter().any(|k| k.len() < 4));
    }
}
