//! Salary range extraction from job description text.
//!
//! Scans for common salary patterns ($120,000-$150,000, £50k-70k,
//! €40,000–60,000, ₹15-25 LPA, $50-70/hr) and returns a structured range.

use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SalaryRange {
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub currency: String,
    pub period: String,
    pub raw_text: String,
}

/// Extract a salary range from a job description. Returns `None` if no
/// recognizable salary pattern is found.
pub fn extract_salary_range(description: &str) -> Option<SalaryRange> {
    // Pattern 1: $120,000 - $150,000 or $120,000–$150,000 or $120k - $150k
    let re_dollar = Regex::new(r"\$(\d[\d,]*)([kK])?\s*[-–—to]+\s*\$?(\d[\d,]*)([kK])?").ok()?;

    // Pattern 2: £50,000 - £70,000
    let re_gbp = Regex::new(r"£(\d[\d,]*)\s*[kK]?\s*[-–—to]+\s*£?(\d[\d,]*)\s*[kK]?").ok()?;

    // Pattern 3: €50,000 - €70,000
    let re_eur = Regex::new(r"€(\d[\d,]*)\s*[kK]?\s*[-–—to]+\s*€?(\d[\d,]*)\s*[kK]?").ok()?;

    // Pattern 4: ₹15,00,000 - ₹25,00,000 or 15 LPA - 25 LPA
    let re_inr = Regex::new(
        r"[₹]?\s*(\d[\d,]*)\s*(?:LPA|lpa|LPA)\s*[-–—to]+\s*[₹]?\s*(\d[\d,]*)\s*(?:LPA|lpa)?",
    )
    .ok()?;

    // Pattern 5: $50-$70/hour or $50-70/hr
    let re_hourly =
        Regex::new(r"\$(\d[\d,]*)\s*[-–—to]+\s*\$?(\d[\d,]*)\s*/\s*(?:hr|hour)").ok()?;

    // Try hourly first (most specific).
    if let Some(caps) = re_hourly.captures(description) {
        let min = parse_num(&caps[1], false);
        let max = parse_num(&caps[2], false);
        let raw = caps
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        return Some(SalaryRange {
            min,
            max,
            currency: "USD".into(),
            period: "hour".into(),
            raw_text: raw,
        });
    }

    // Try dollar.
    if let Some(caps) = re_dollar.captures(description) {
        let min_k = caps.get(2).is_some();
        let max_k = caps.get(4).is_some();
        let has_k = min_k || max_k;
        let min = parse_num(&caps[1], has_k);
        let max = parse_num(&caps[3], has_k);
        let raw = caps
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        return Some(SalaryRange {
            min,
            max,
            currency: "USD".into(),
            period: "year".into(),
            raw_text: raw,
        });
    }

    // Try GBP.
    if let Some(caps) = re_gbp.captures(description) {
        let min = parse_num(&caps[1], false);
        let max = parse_num(&caps[2], false);
        let raw = caps
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        return Some(SalaryRange {
            min,
            max,
            currency: "GBP".into(),
            period: "year".into(),
            raw_text: raw,
        });
    }

    // Try EUR.
    if let Some(caps) = re_eur.captures(description) {
        let min = parse_num(&caps[1], false);
        let max = parse_num(&caps[2], false);
        let raw = caps
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        return Some(SalaryRange {
            min,
            max,
            currency: "EUR".into(),
            period: "year".into(),
            raw_text: raw,
        });
    }

    // Try INR LPA.
    if let Some(caps) = re_inr.captures(description) {
        let min = parse_num(&caps[1], false);
        let max = parse_num(&caps[2], false);
        let raw = caps
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        return Some(SalaryRange {
            min,
            max,
            currency: "INR".into(),
            period: "year".into(),
            raw_text: raw,
        });
    }

    None
}

/// Parse a number string, stripping commas. If `multiply_k` is true,
/// multiply by 1000 (for "120k" style notation).
fn parse_num(s: &str, multiply_k: bool) -> Option<u64> {
    let cleaned: String = s.chars().filter(char::is_ascii_digit).collect();
    let n: u64 = cleaned.parse().ok()?;
    if multiply_k {
        Some(n * 1000)
    } else {
        Some(n)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_dollar_range_with_commas() {
        let s = extract_salary_range("Salary: $120,000 - $150,000 per year").unwrap();
        assert_eq!(s.min, Some(120_000));
        assert_eq!(s.max, Some(150_000));
        assert_eq!(s.currency, "USD");
        assert_eq!(s.period, "year");
    }

    #[test]
    fn test_dollar_range_with_k_suffix() {
        let s = extract_salary_range("Paying $120k - $150k").unwrap();
        assert_eq!(s.min, Some(120_000));
        assert_eq!(s.max, Some(150_000));
    }

    #[test]
    fn test_hourly_rate() {
        let s = extract_salary_range("$50-$70/hour DOE").unwrap();
        assert_eq!(s.min, Some(50));
        assert_eq!(s.max, Some(70));
        assert_eq!(s.period, "hour");
    }

    #[test]
    fn test_gbp_range() {
        let s = extract_salary_range("£50,000 - £70,000").unwrap();
        assert_eq!(s.currency, "GBP");
        assert_eq!(s.min, Some(50_000));
    }

    #[test]
    fn test_no_salary_mentioned() {
        assert!(extract_salary_range("Great company, amazing benefits!").is_none());
    }

    #[test]
    fn test_inr_lpa() {
        let s = extract_salary_range("15 LPA - 25 LPA").unwrap();
        assert_eq!(s.currency, "INR");
        assert_eq!(s.min, Some(15));
        assert_eq!(s.max, Some(25));
    }
}
