//! Per-call LLM cost tracking.
//!
//! Each live provider call produces a [`CostRecord`] with token counts and
//! an estimated USD cost. [`CostTracker`] accumulates records in memory and
//! appends them as JSONL to `data/cache/llm/costs.jsonl` so costs survive
//! restarts. Cost estimation uses simple per-1K-token rates keyed by model
//! name substring; models without a known rate fall back to $1/$5.
//!
//! Cache hits (our on-disk [`Cache`]) do not incur provider cost, so the
//! cache layer strips `cost` before persisting — see [`crate::cache`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::error::Result;

/// A single LLM call's cost record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    pub model: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

/// Per-1K-token pricing (input, output) for a model family.
struct Pricing {
    input_per_1k: f64,
    output_per_1k: f64,
}

/// Estimate USD cost for a call.
///
/// `input_tokens` are non-cached input tokens billed at the model's full
/// input rate. `cache_read_tokens` are input tokens served from the
/// provider's prompt cache, billed at 10 % of the input rate (Anthropic's
/// cache-read pricing). `output_tokens` are billed at the full output
/// rate.
///
/// # Examples
///
/// ```
/// use careerai_llm::estimate_cost;
/// // 1K input + 500 output on Sonnet → 3.0 + 7.5 = 10.5
/// let cost = estimate_cost("claude-3-5-sonnet", 1000, 500, 0);
/// assert!((cost - 10.5).abs() < 1e-9);
/// ```
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    let pricing = pricing_for(model);
    // Token counts are well below 2^52, so the cast is lossless in practice.
    let input_cost = (input_tokens as f64 / 1000.0) * pricing.input_per_1k;
    let cache_read_cost = (cache_read_tokens as f64 / 1000.0) * pricing.input_per_1k * 0.1;
    let output_cost = (output_tokens as f64 / 1000.0) * pricing.output_per_1k;
    input_cost + cache_read_cost + output_cost
}

/// Resolve per-1K-token pricing by model name substring.
///
/// - Claude Sonnet → $3 / $15
/// - GPT-4o-mini  → $0.15 / $0.60
/// - GPT-4o       → $2.50 / $10
/// - anything else → $1 / $5 (conservative default)
fn pricing_for(model: &str) -> Pricing {
    let lower = model.to_ascii_lowercase();
    if lower.contains("gpt-4o-mini") {
        Pricing {
            input_per_1k: 0.15,
            output_per_1k: 0.60,
        }
    } else if lower.contains("sonnet") {
        Pricing {
            input_per_1k: 3.0,
            output_per_1k: 15.0,
        }
    } else if lower.contains("gpt-4o") {
        Pricing {
            input_per_1k: 2.50,
            output_per_1k: 10.0,
        }
    } else if lower.contains("glm") {
        Pricing {
            input_per_1k: 0.001,
            output_per_1k: 0.002,
        }
    } else {
        Pricing {
            input_per_1k: 1.0,
            output_per_1k: 5.0,
        }
    }
}

/// Accumulates LLM costs in memory and persists each record as a line in
/// an append-only JSONL file.
///
/// The in-memory `total` reflects every call to [`record`](Self::record)
/// since the tracker was created. [`total_cost_from_disk`](Self::total_cost_from_disk)
/// reads the JSONL file from scratch, useful after a restart.
#[derive(Debug)]
pub struct CostTracker {
    path: PathBuf,
    total: f64,
}

impl CostTracker {
    /// Create a tracker that writes to `path` (typically
    /// `data/cache/llm/costs.jsonl`).
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            total: 0.0,
        }
    }

    /// Append a record to the JSONL file and add its cost to the running
    /// total. Creates parent directories as needed.
    pub async fn record(&mut self, record: CostRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string(&record)
            .map_err(|e| crate::error::LlmError::Schema(e.to_string()))?;
        let line = format!("{json}\n");
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        self.total += record.cost_usd;
        Ok(())
    }

    /// Running total of all costs recorded via [`record`](Self::record)
    /// since this tracker was created.
    #[must_use]
    pub fn total_cost(&self) -> f64 {
        self.total
    }

    /// Sum every `cost_usd` field in the JSONL file. Returns `0.0` if the
    /// file does not exist yet.
    pub async fn total_cost_from_disk(&self) -> Result<f64> {
        use tokio::io::AsyncBufReadExt;
        match tokio::fs::File::open(&self.path).await {
            Ok(file) => {
                let reader = tokio::io::BufReader::new(file);
                let mut lines = reader.lines();
                let mut total = 0.0;
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(record) = serde_json::from_str::<CostRecord>(&line) {
                        total += record.cost_usd;
                    }
                }
                Ok(total)
            }
            Err(_) => Ok(0.0),
        }
    }

    /// Sum `cost_usd` for records whose `timestamp` falls on the current
    /// UTC calendar day. Used by the F07 budget enforcement gate.
    pub async fn daily_cost_from_disk(&self) -> Result<f64> {
        let today = chrono::Utc::now().date_naive();
        self.sum_filtered(|ts| ts.date_naive() == today).await
    }

    /// Count records whose `timestamp` falls on the current UTC calendar
    /// day. Used by the F07 call-cap enforcement gate.
    pub async fn daily_call_count_from_disk(&self) -> Result<u64> {
        use tokio::io::AsyncBufReadExt;
        let today = chrono::Utc::now().date_naive();
        match tokio::fs::File::open(&self.path).await {
            Ok(file) => {
                let reader = tokio::io::BufReader::new(file);
                let mut lines = reader.lines();
                let mut count = 0u64;
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(record) = serde_json::from_str::<CostRecord>(&line) {
                        if record.timestamp.date_naive() == today {
                            count += 1;
                        }
                    }
                }
                Ok(count)
            }
            Err(_) => Ok(0),
        }
    }

    async fn sum_filtered<F>(&self, predicate: F) -> Result<f64>
    where
        F: Fn(&chrono::DateTime<chrono::Utc>) -> bool,
    {
        use tokio::io::AsyncBufReadExt;
        match tokio::fs::File::open(&self.path).await {
            Ok(file) => {
                let reader = tokio::io::BufReader::new(file);
                let mut lines = reader.lines();
                let mut total = 0.0;
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(record) = serde_json::from_str::<CostRecord>(&line) {
                        if predicate(&record.timestamp) {
                            total += record.cost_usd;
                        }
                    }
                }
                Ok(total)
            }
            Err(_) => Ok(0.0),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // --- estimate_cost ------------------------------------------------

    #[test]
    fn estimate_cost_sonnet() {
        // 1K input @ $3 + 500 output @ $15 = 3.0 + 7.5 = 10.5
        let cost = estimate_cost("claude-3-5-sonnet", 1000, 500, 0);
        assert!((cost - 10.5).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_gpt_4o() {
        // 1K input @ $2.50 + 500 output @ $10 = 2.5 + 5.0 = 7.5
        let cost = estimate_cost("gpt-4o", 1000, 500, 0);
        assert!((cost - 7.5).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_gpt_4o_mini() {
        // 1K input @ $0.15 + 500 output @ $0.60 = 0.15 + 0.30 = 0.45
        let cost = estimate_cost("gpt-4o-mini", 1000, 500, 0);
        assert!((cost - 0.45).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_unknown_defaults_to_one_five() {
        // 1K input @ $1 + 500 output @ $5 = 1.0 + 2.5 = 3.5
        let cost = estimate_cost("some-unknown-model", 1000, 500, 0);
        assert!((cost - 3.5).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_gpt_4o_mini_takes_precedence_over_gpt_4o() {
        // "gpt-4o-mini" contains "gpt-4o", so the mini check must win.
        let cost = estimate_cost("gpt-4o-mini", 1000, 500, 0);
        assert!((cost - 0.45).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_glm() {
        // 1K input @ $0.001 + 500 output @ $0.002 = 0.001 + 0.001 = 0.002
        let cost = estimate_cost("zai-org-glm-52-fp8", 1000, 500, 0);
        assert!((cost - 0.002).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_with_cache_read_discount() {
        // 1K input @ $3 + 500 cache-read @ $0.30 + 500 output @ $15
        // = 3.0 + 0.15 + 7.5 = 10.65
        let cost = estimate_cost("claude-sonnet", 1000, 500, 500);
        assert!((cost - 10.65).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn estimate_cost_zero_tokens() {
        let cost = estimate_cost("claude-sonnet", 0, 0, 0);
        assert!((cost - 0.0).abs() < 1e-9);
    }

    // --- CostTracker ---------------------------------------------------

    fn sample_record(cost: f64) -> CostRecord {
        CostRecord {
            model: "claude-sonnet".into(),
            timestamp: chrono::Utc::now(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cost_usd: cost,
        }
    }

    #[tokio::test]
    async fn tracker_records_and_sums_in_memory() {
        let dir = tempdir().unwrap();
        let mut tracker = CostTracker::new(dir.path().join("costs.jsonl"));

        tracker.record(sample_record(10.5)).await.unwrap();
        assert!((tracker.total_cost() - 10.5).abs() < 1e-9);

        tracker.record(sample_record(6.0)).await.unwrap();
        assert!((tracker.total_cost() - 16.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn tracker_persists_jsonl() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("costs.jsonl");
        let mut tracker = CostTracker::new(path.clone());

        tracker.record(sample_record(10.5)).await.unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1, "expected exactly one JSONL line");
        let parsed: CostRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.model, "claude-sonnet");
        assert!((parsed.cost_usd - 10.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn tracker_appends_multiple_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("costs.jsonl");
        let mut tracker = CostTracker::new(path.clone());

        tracker.record(sample_record(10.5)).await.unwrap();
        tracker.record(sample_record(6.0)).await.unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2, "expected two JSONL lines");
    }

    #[tokio::test]
    async fn tracker_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("costs.jsonl");
        let mut tracker = CostTracker::new(path.clone());

        tracker.record(sample_record(1.0)).await.unwrap();
        assert!(path.exists(), "file should have been created");
    }

    #[tokio::test]
    async fn tracker_total_from_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("costs.jsonl");
        let mut tracker = CostTracker::new(path.clone());

        tracker.record(sample_record(10.5)).await.unwrap();
        tracker.record(sample_record(6.0)).await.unwrap();

        // Fresh tracker reads from disk.
        let tracker2 = CostTracker::new(path);
        let total = tracker2.total_cost_from_disk().await.unwrap();
        assert!((total - 16.5).abs() < 1e-9, "got {total}");
    }

    #[tokio::test]
    async fn tracker_total_from_disk_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.jsonl");
        let tracker = CostTracker::new(path);
        let total = tracker.total_cost_from_disk().await.unwrap();
        assert!((total - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn tracker_daily_cost_excludes_old_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("costs.jsonl");

        // Yesterday's record — must be excluded from today's total.
        let old = CostRecord {
            model: "claude-sonnet".into(),
            timestamp: chrono::Utc::now() - chrono::Duration::days(1),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cost_usd: 100.0,
        };
        // Today's record — must be included.
        let today = CostRecord {
            model: "claude-sonnet".into(),
            timestamp: chrono::Utc::now(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cost_usd: 5.0,
        };

        let mut tracker = CostTracker::new(path.clone());
        tracker.record(old).await.unwrap();
        tracker.record(today).await.unwrap();

        let daily = tracker.daily_cost_from_disk().await.unwrap();
        assert!(
            (daily - 5.0).abs() < 1e-9,
            "daily cost should be 5.0 (today only), got {daily}"
        );
    }

    #[tokio::test]
    async fn tracker_daily_count_excludes_old_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("costs.jsonl");

        let old = sample_record(1.0);
        let mut tracker = CostTracker::new(path.clone());
        tracker
            .record(CostRecord {
                timestamp: chrono::Utc::now() - chrono::Duration::days(1),
                ..old
            })
            .await
            .unwrap();
        tracker.record(sample_record(2.0)).await.unwrap();

        let count = tracker.daily_call_count_from_disk().await.unwrap();
        assert_eq!(count, 1, "daily call count should be 1 (today only)");
    }
    // --- CostRecord serialization -------------------------------------

    #[test]
    fn cost_record_serializes_roundtrip() {
        let record = CostRecord {
            model: "claude-sonnet".into(),
            timestamp: chrono::Utc::now(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cost_usd: 10.5,
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: CostRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model, record.model);
        assert_eq!(parsed.input_tokens, record.input_tokens);
        assert_eq!(parsed.output_tokens, record.output_tokens);
        assert_eq!(parsed.cache_read_tokens, record.cache_read_tokens);
        assert!((parsed.cost_usd - record.cost_usd).abs() < 1e-9);
    }
}
