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
        let tier = l.score.map_or_else(String::new, |s| {
            let cfg = careerai_core::config::CoreConfig::load(cwd).ok();
            match cfg {
                // DB stores f64; tier boundaries are f32.
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                Some(c) => {
                    format!(
                        " [{}]",
                        careerai_match::tier_for(s as f32, &c.matching).label()
                    )
                }
                None => String::new(),
            }
        });
        println!(
            "{:>2}. [{score}]{tier} {} @ {} ({})\n    id:   {}\n    url:  {}",
            i + 1,
            l.title,
            l.company,
            l.source,
            l.id,
            l.url,
        );
    }
    Ok(())
}
