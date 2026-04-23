//! Shared date normalization for resume sources.
//!
//! All parsers should funnel start/end dates through `normalize` so the
//! merge layer's `(company, title, start)` dedupe key is comparable across
//! PDF, DOCX, and LinkedIn sources.
//!
//! Output grammar:
//! - `"present"` for empty, unset, or "current"-shaped inputs on the end side
//!   (caller decides which side to interpret as present).
//! - `"YYYY-MM"` when month + year detected (e.g. `"Jan 2022"` → `"2022-01"`,
//!   `"2022-01"` → `"2022-01"`, `"2022/01"` → `"2022-01"`).
//! - `"YYYY"` when only year detected.
//! - The trimmed input verbatim if no shape matched.

/// Normalize a single date token to `YYYY-MM` / `YYYY` / `""` / verbatim.
#[must_use]
pub fn normalize(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Already in YYYY-MM or YYYY/MM form.
    if let Some((year, month)) = trimmed.split_once(['-', '/']) {
        if year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) {
            if let Some(mm) = two_digit_month(month) {
                return format!("{year}-{mm}");
            }
            // YYYY-only with a junk suffix → keep year.
            if month.is_empty() {
                return year.to_string();
            }
        }
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    match parts.as_slice() {
        [year] if is_year(year) => (*year).to_string(),
        [month, year] if is_year(year) => match month_to_number(month) {
            Some(mm) => format!("{year}-{mm}"),
            // Unknown month token (e.g. "Q1 2022", "FY 2022") — preserve the
            // original verbatim rather than silently dropping the qualifier.
            None => trimmed.to_string(),
        },
        _ => trimmed.to_string(),
    }
}

fn is_year(s: &str) -> bool {
    s.len() == 4 && s.chars().all(|c| c.is_ascii_digit())
}

fn two_digit_month(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if !(1..=2).contains(&trimmed.len()) || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month: u8 = trimmed.parse().ok()?;
    if (1..=12).contains(&month) {
        Some(format!("{month:02}"))
    } else {
        None
    }
}

pub(crate) fn month_to_number(month: &str) -> Option<&'static str> {
    let lower = month.to_lowercase();
    match lower.get(..3)? {
        "jan" => Some("01"),
        "feb" => Some("02"),
        "mar" => Some("03"),
        "apr" => Some("04"),
        "may" => Some("05"),
        "jun" => Some("06"),
        "jul" => Some("07"),
        "aug" => Some("08"),
        "sep" => Some("09"),
        "oct" => Some("10"),
        "nov" => Some("11"),
        "dec" => Some("12"),
        _ => None,
    }
}

/// Treat the end-of-tenure side: empty + "present"/"current"/"now" map to "present",
/// everything else passes through `normalize`.
#[must_use]
pub fn normalize_end(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "present".to_string();
    }
    let lower = trimmed.to_lowercase();
    if matches!(lower.as_str(), "present" | "current" | "now" | "today") {
        return "present".to_string();
    }
    normalize(raw)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_linkedin_style() {
        assert_eq!(normalize("Feb 2022"), "2022-02");
        assert_eq!(normalize("December 2019"), "2019-12");
        assert_eq!(normalize("2020"), "2020");
    }

    #[test]
    fn normalizes_iso_style() {
        assert_eq!(normalize("2022-01"), "2022-01");
        assert_eq!(normalize("2022/1"), "2022-01");
        assert_eq!(normalize("2022/01"), "2022-01");
    }

    #[test]
    fn empty_passes_through() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn unrecognized_passes_through_verbatim() {
        assert_eq!(normalize("Q1 2022"), "Q1 2022");
        assert_eq!(normalize("Spring 2020"), "Spring 2020");
    }

    #[test]
    fn out_of_range_month_passes_through_verbatim() {
        // Month 0 and 13 are not valid; normalize should not produce YYYY-MM.
        assert_eq!(normalize("2022-0"), "2022-0");
        assert_eq!(normalize("2022-13"), "2022-13");
        assert_eq!(normalize("2022/00"), "2022/00");
    }

    #[test]
    fn end_side_maps_present_synonyms() {
        assert_eq!(normalize_end(""), "present");
        assert_eq!(normalize_end("Present"), "present");
        assert_eq!(normalize_end("CURRENT"), "present");
        assert_eq!(normalize_end("now"), "present");
        assert_eq!(normalize_end("Jan 2025"), "2025-01");
    }
}
