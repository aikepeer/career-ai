//! Batched LLM helpers for optional hybrid fallback work.

use std::collections::HashMap;

use careerai_core::config::LlmConfig;
use careerai_llm::{Llm, LlmRequest};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TailorError};

#[derive(Debug, Clone, Serialize)]
pub struct SummaryJob {
    pub id: String,
    pub job_description: String,
    pub current_summary: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SummaryResult {
    pub id: String,
    pub summary: String,
}

/// Reword summaries in batches, retrying missing or malformed items singly.
pub async fn reword_summaries(
    jobs: &[SummaryJob],
    profile_block: &str,
    llm: &(dyn Llm + Send + Sync),
    cfg: &LlmConfig,
) -> Result<Vec<SummaryResult>> {
    let batch_size = cfg.batch_size.max(1);
    let mut results = HashMap::with_capacity(jobs.len());
    for chunk in jobs.chunks(batch_size) {
        let request = batch_request(chunk, profile_block, cfg);
        let response = llm.complete(&request).await?;
        let parsed = parse_results(&response.text).unwrap_or_default();
        for result in parsed {
            if chunk.iter().any(|job| job.id == result.id) && !result.summary.trim().is_empty() {
                results.insert(result.id.clone(), result);
            }
        }
        for job in chunk {
            if results.contains_key(&job.id) {
                continue;
            }
            let response = llm
                .complete(&single_request(job, profile_block, cfg))
                .await?;
            let result = parse_results(&response.text)
                .ok_or_else(|| {
                    TailorError::Schema(format!("missing summary result for {}", job.id))
                })?
                .into_iter()
                .find(|result| result.id == job.id)
                .ok_or_else(|| {
                    TailorError::Schema(format!("wrong summary result for {}", job.id))
                })?;
            results.insert(job.id.clone(), result);
        }
    }
    jobs.iter()
        .map(|job| {
            results
                .remove(&job.id)
                .ok_or_else(|| TailorError::Schema(format!("no summary result for {}", job.id)))
        })
        .collect()
}

fn batch_request(jobs: &[SummaryJob], profile_block: &str, cfg: &LlmConfig) -> LlmRequest {
    LlmRequest {
        system: "Return only a JSON array of {id,summary}. Reword each summary using only facts already present in the profile and job description. Never invent metrics, employers, dates, or technologies.".into(),
        profile_block: profile_block.to_string(),
        user: serde_json::to_string(jobs).unwrap_or_else(|_| "[]".into()),
        prompt_version: "summary-batch.v1".into(),
        model: if cfg.tailor_model.is_empty() {
            cfg.model.clone()
        } else {
            cfg.tailor_model.clone()
        },
        max_tokens: 2_000,
        temperature: 0.2,
        cache_profile: cfg.anthropic_prompt_cache,
    }
}

fn single_request(job: &SummaryJob, profile_block: &str, cfg: &LlmConfig) -> LlmRequest {
    batch_request(std::slice::from_ref(job), profile_block, cfg)
}

fn parse_results(raw: &str) -> Option<Vec<SummaryResult>> {
    let json = raw
        .strip_prefix("```json")
        .and_then(|text| text.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(raw.trim());
    serde_json::from_str(json).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_batch_json_and_rejects_malformed_payloads() {
        let parsed = parse_results(r#"[{"id":"a","summary":"Rust systems"}]"#).unwrap();
        assert_eq!(parsed[0].id, "a");
        assert!(parse_results("not json").is_none());
    }
}
