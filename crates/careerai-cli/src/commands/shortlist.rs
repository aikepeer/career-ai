//! `careerai shortlist show` — list shortlisted listings with score + URL.

use std::path::Path;

use anyhow::Result;

use careerai_pipeline as pipeline;

pub async fn run_show(cwd: &Path, limit: u32) -> Result<()> {
    let rows = pipeline::shortlist_show(cwd, i64::from(limit)).await?;
    if rows.is_empty() {
        println!("(no shortlisted listings — run `careerai discover` then `careerai match`)");
        return Ok(());
    }
    for (i, l) in rows.iter().enumerate() {
        let score = l
            .score
            .map_or_else(|| "—".to_string(), |s| format!("{s:.3}"));
        println!(
            "{:>2}. [{score}] {} @ {} ({})\n    {}",
            i + 1,
            l.title,
            l.company,
            l.source,
            l.url,
        );
    }
    Ok(())
}
