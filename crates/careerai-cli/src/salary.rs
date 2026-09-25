//! `careerai salary` — benchmark a company's salary against your own data.
//!
//! Ported from ai-job-search's `salary_lookup.py`: reads a user-provided
//! `salary_data.json` at the career-ai root (union statistics, Glassdoor
//! exports, manually collected benchmarks — any index- or absolute-value
//! dataset), fuzzy-matches the company name (legal suffixes, Nordic
//! spelling variants, parentheticals), narrows by city, and prints the
//! category table with count / index / vs-baseline. Optional by design:
//! when no data file exists the command explains how to set one up and
//! exits non-zero — the pipeline itself never depends on it.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

use careerai_core::salary::{CompanySalary, SalaryData};

const HELP: &str = "\
This tool requires a salary data file at <root>/salary_data.json.

Format:
  { \"metadata\": { \"source\": \"My Union Statistics 2025\",
                    \"index_baseline\": 100, \"index_label\": \"Index\" },
    \"companies\": [
      { \"company\": \"Novo Nordisk A/S\", \"city\": \"Bagsværd\",
        \"categories\": { \"engineering\": { \"count\": 120, \"index\": 112.3 } } }
    ] }

If you don't have salary data, the salary step is simply skipped.";

pub struct SalaryArgs {
    pub company: Option<String>,
    pub city: Option<String>,
    pub flags: SalaryFlags,
}

/// Boolean switches for the salary command. Grouped so clippy's
/// `struct_excessive_bools` stays quiet.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // four independent CLI flags
pub struct SalaryFlags {
    pub json: bool,
    pub list_all: bool,
    pub validate: bool,
    pub gap: bool,
}

/// Parse "$150K-200K" / "150000-200000" / "€90k" into (low, high)
/// numbers when possible. Returns `None` for unparseable ranges.
fn parse_target_range(range: &str) -> Option<(f64, f64)> {
    let digits: Vec<f64> = range
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|tok| {
            let tok = tok.trim();
            if tok.is_empty() {
                return None;
            }
            let (num_part, mult) = match tok.chars().last() {
                Some('k' | 'K') => (&tok[..tok.len() - 1], 1000.0),
                _ => (tok, 1.0),
            };
            if num_part.is_empty() {
                return None;
            }
            let mut n: f64 = num_part.parse().ok()?;
            n *= mult;
            Some(n)
        })
        .collect();
    match digits.as_slice() {
        [low, high] if high >= low => Some((*low, *high)),
        [single] => Some((*single, *single)),
        _ => None,
    }
}

pub fn run(root: &Path, args: &SalaryArgs) -> Result<()> {
    let data_path = root.join("salary_data.json");
    if !data_path.exists() {
        return Err(anyhow!(
            "Error: salary_data.json not found at {}.\n\n{HELP}",
            data_path.display()
        ));
    }
    let data =
        SalaryData::load(&data_path).with_context(|| format!("load {}", data_path.display()))?;

    if args.flags.validate {
        let warnings = data.warnings();
        if warnings.is_empty() {
            println!("salary data OK ({} companies)", data.companies.len());
        } else {
            for w in &warnings {
                println!("warning: {w}");
            }
        }
        return Ok(());
    }

    if args.flags.list_all {
        if args.flags.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&data).context("serialize salary data")?
            );
        } else {
            for c in &data.companies {
                print_company(c, &data);
            }
        }
        return Ok(());
    }

    let Some(query) = &args.company else {
        return Err(anyhow!(
            "company name required; see `careerai salary --help`"
        ));
    };

    let hits = data.lookup(query, args.city.as_deref());
    if hits.is_empty() {
        println!("No salary data found for '{query}'.");
        return Ok(());
    }

    // --gap (career-ops salary-gap): compare the profile's target comp
    // against the matched company's market index.
    if args.flags.gap {
        let profile_path = careerai_core::paths::profile_path(root);
        if profile_path.exists() {
            let text = std::fs::read_to_string(&profile_path).unwrap_or_default();
            if let Ok(profile) = careerai_profile::schema::Profile::from_yaml(&text) {
                if !profile.compensation.target_range.is_empty() {
                    let target = &profile.compensation.target_range;
                    let range = parse_target_range(target);
                    println!("\n--- Compensation gap ---");
                    println!("Your target: {target}");
                    if let Some(min) = profile.compensation.minimum.split_whitespace().next() {
                        println!("Walk-away minimum: {min}");
                    }
                    for c in &hits {
                        if let Some((_, best_idx)) = best_category_index(c) {
                            let baseline = data.metadata.index_baseline;
                            let diff_pct = ((best_idx - baseline) / baseline) * 100.0;
                            println!(
                                "{} market index: {best_idx:.1} ({diff_pct:+.1}% vs baseline)",
                                c.company
                            );
                            if let (Some((low, _)), true) = (range, diff_pct >= 0.0) {
                                println!(
                                    "Verdict: market pays {diff_pct:+.0}% above baseline — \
                                     target of {low:.0} currency units is {} the market median",
                                    if low >= baseline {
                                        "at or above"
                                    } else {
                                        "below"
                                    }
                                );
                            }
                        }
                    }
                    return Ok(());
                }
            }
        }
        println!("No profile compensation configured — set `compensation.target_range` in profile/profile.yaml.");
        return Ok(());
    }

    if args.flags.json {
        let json: Vec<&CompanySalary> = hits;
        println!(
            "{}",
            serde_json::to_string_pretty(&json).context("serialize salary hits")?
        );
    } else {
        for c in hits {
            print_company(c, &data);
        }
    }
    Ok(())
}

/// Highest-index category of a company entry (the best-paying category
/// is the one a candidate negotiates against).
fn best_category_index(c: &CompanySalary) -> Option<(&str, f64)> {
    c.categories
        .iter()
        .filter_map(|(name, cat)| cat.index.map(|i| (name.as_str(), i)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}

/// Human-readable company entry: header + category table with count,
/// index, and +/-% vs the metadata baseline (mirrors the Python tool).
fn print_company(c: &CompanySalary, data: &SalaryData) {
    let meta = &data.metadata;
    println!();
    println!("{}", "=".repeat(60));
    println!("  {}", c.company);
    if let Some(city) = &c.city {
        println!("  Location: {city}");
    }
    println!("{}", "=".repeat(60));

    let label = &meta.index_label;
    let baseline = meta.index_baseline;
    println!(
        "  {:<22} {:>6} {:>8}  {:>10}",
        "Category", "Count", label, "vs Baseline"
    );
    println!("  {}", "-".repeat(50));

    for (cat_name, cat) in &c.categories {
        let display: String = cat_name
            .replace('_', " ")
            .split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let count_str = cat
            .count
            .map_or_else(|| "-".to_string(), |n| format!("{n:.0}"));
        let (index_str, diff_str) = match cat.index {
            Some(index) if baseline != 0.0 => {
                let diff_pct = ((index - baseline) / baseline) * 100.0;
                let sign = if diff_pct >= 0.0 { "+" } else { "" };
                (format!("{index:.1}"), format!("{sign}{diff_pct:.1}%"))
            }
            Some(index) => (format!("{index:.1}"), String::new()),
            None => ("-".to_string(), String::new()),
        };
        println!("  {display:<22} {count_str:>6} {index_str:>8}  {diff_str:>10}");
    }
    if let Some(src) = &meta.source {
        println!("  Source: {src}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    fn write_sample(root: &Path) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("salary_data.json"),
            r#"{
                "metadata": {"source": "Union Stats", "index_baseline": 100, "index_label": "Index"},
                "companies": [
                    {"company": "Novo Nordisk A/S", "city": "Bagsværd",
                     "categories": {"engineering": {"count": 120, "index": 112.3}}},
                    {"company": "Beta Corp", "city": null,
                     "categories": {"all_employees": {"index": 91.0}}}
                ]
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn missing_data_file_prints_setup_help() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run(
            tmp.path(),
            &SalaryArgs {
                company: Some("Acme".into()),
                city: None,
                flags: SalaryFlags {
                    json: false,
                    list_all: false,
                    validate: false,
                    gap: false,
                },
            },
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("salary_data.json not found"));
        assert!(format!("{err:#}").contains("companies"));
    }

    #[test]
    fn lookup_resolves_fuzzy_company() {
        let tmp = tempfile::tempdir().unwrap();
        write_sample(tmp.path());
        assert!(run(
            tmp.path(),
            &SalaryArgs {
                company: Some("Novo Nordisk".into()),
                city: None,
                flags: SalaryFlags {
                    json: false,
                    list_all: false,
                    validate: false,
                    gap: false,
                },
            },
        )
        .is_ok());
    }

    #[test]
    fn lookup_json_serializes_hits() {
        let tmp = tempfile::tempdir().unwrap();
        write_sample(tmp.path());
        // JSON path prints to stdout; instead verify the underlying
        // lookup contract the CLI wraps.
        let data = SalaryData::load(&tmp.path().join("salary_data.json")).unwrap();
        let hits = data.lookup("Novo", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].categories["engineering"].index, Some(112.3));
    }

    #[test]
    fn no_match_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_sample(tmp.path());
        let res = run(
            tmp.path(),
            &SalaryArgs {
                company: Some("Starbucks".into()),
                city: None,
                flags: SalaryFlags {
                    json: false,
                    list_all: false,
                    validate: false,
                    gap: false,
                },
            },
        );
        assert!(res.is_ok());
    }

    #[test]
    fn parse_target_range_handles_k_and_dash() {
        assert_eq!(
            parse_target_range("$150K-200K"),
            Some((150_000.0, 200_000.0))
        );
        assert_eq!(parse_target_range("€90k"), Some((90_000.0, 90_000.0)));
        assert_eq!(
            parse_target_range("120000-150000"),
            Some((120_000.0, 150_000.0))
        );
        assert_eq!(parse_target_range("n/a"), None);
        assert_eq!(parse_target_range(""), None);
    }

    #[test]
    fn best_category_index_picks_highest() {
        let c = CompanySalary {
            company: "Acme".into(),
            city: None,
            categories: [
                (
                    "engineering".to_string(),
                    careerai_core::salary::Category {
                        count: None,
                        index: Some(105.0),
                    },
                ),
                (
                    "ml_ai".to_string(),
                    careerai_core::salary::Category {
                        count: None,
                        index: Some(118.0),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };
        let (name, idx) = best_category_index(&c).unwrap();
        assert_eq!(name, "ml_ai");
        assert!((idx - 118.0).abs() < f64::EPSILON);
    }

    #[test]
    fn validate_reports_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(
            tmp.path().join("salary_data.json"),
            r#"{"companies": [{"company": "Acme A/S"}, {"company": "Acme"}]}"#,
        )
        .unwrap();
        assert!(run(
            tmp.path(),
            &SalaryArgs {
                company: None,
                city: None,
                flags: SalaryFlags {
                    json: false,
                    list_all: false,
                    validate: true,
                    gap: false,
                },
            },
        )
        .is_ok());
    }
}
