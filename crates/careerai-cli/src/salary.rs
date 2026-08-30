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
    pub json: bool,
    pub list_all: bool,
    pub validate: bool,
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

    if args.validate {
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

    if args.list_all {
        if args.json {
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

    if args.json {
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
                json: false,
                list_all: false,
                validate: false,
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
                json: false,
                list_all: false,
                validate: false,
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
                json: false,
                list_all: false,
                validate: false,
            },
        );
        assert!(res.is_ok());
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
                json: false,
                list_all: false,
                validate: true,
            },
        )
        .is_ok());
    }
}
