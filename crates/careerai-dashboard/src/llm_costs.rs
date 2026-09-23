//! Read the LLM cost JSONL log and aggregate for the dashboard.
//!
//! The cost file lives in the XDG cache directory unless an explicit project
//! root is configured.

#![allow(clippy::cast_precision_loss)]

use std::path::PathBuf;

use crate::view::LlmCostSummary;

/// Read `costs.jsonl` and return an aggregate summary. Returns a
/// zeroed summary if the file does not exist yet (no LLM calls made).
pub async fn cost_summary(base_dir: &std::path::Path) -> LlmCostSummary {
    let path: PathBuf = careerai_core::paths::cache_dir_for_root(base_dir)
        .join("llm")
        .join("costs.jsonl");
    read_cost_file(&path).await
}

async fn read_cost_file(path: &std::path::Path) -> LlmCostSummary {
    use tokio::io::AsyncBufReadExt;

    let Ok(file) = tokio::fs::File::open(path).await else {
        return LlmCostSummary {
            total_cost_usd: 0.0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            call_count: 0,
            model: String::new(),
            estimated_savings_usd: 0.0,
            today_cost_usd: 0.0,
            today_call_count: 0,
        };
    };

    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();

    let mut total_cost = 0.0;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_cache_read: u64 = 0;
    let mut call_count: u64 = 0;
    let mut last_model = String::new();
    let today = chrono::Utc::now().date_naive();
    let mut today_cost = 0.0;
    let mut today_calls: u64 = 0;

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<careerai_llm::CostRecord>(&line) {
            total_cost += record.cost_usd;
            total_input += record.input_tokens;
            total_output += record.output_tokens;
            total_cache_read += record.cache_read_tokens;
            call_count += 1;
            last_model.clone_from(&record.model);
            if record.timestamp.date_naive() == today {
                today_cost += record.cost_usd;
                today_calls += 1;
            }
        }
    }

    // Estimate savings: cache-read tokens would have been billed at the
    // full input rate without caching. This is a lower bound — it does
    // not include similarity-reuse or content-library-reuse savings
    // (those calls were skipped entirely, so there is no cost record).
    let pricing = pricing_for(&last_model);
    let cache_savings = (total_cache_read as f64 / 1000.0) * pricing * 0.9; // 90% discount

    LlmCostSummary {
        total_cost_usd: total_cost,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_cache_read_tokens: total_cache_read,
        call_count,
        model: last_model,
        estimated_savings_usd: cache_savings,
        today_cost_usd: today_cost,
        today_call_count: today_calls,
    }
}

/// Resolve per-1K-token input pricing by model name substring.
/// Mirrors `careerai_llm::cost::pricing_for` but returns just the
/// input rate (needed for cache-savings estimation).
fn pricing_for(model: &str) -> f64 {
    let lower = model.to_ascii_lowercase();
    if lower.contains("gpt-4o-mini") {
        0.15
    } else if lower.contains("sonnet") {
        3.0
    } else if lower.contains("gpt-4o") {
        2.50
    } else if lower.contains("glm") {
        0.001
    } else {
        1.0
    }
}
