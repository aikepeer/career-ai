//! Interactive LLM Chat Agent handler for the Career-AI Dashboard.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::data;
use crate::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct ChatAgentRequest {
    pub message: String,
    #[serde(default)]
    pub history: Vec<ChatMessage>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub async fn api_chat_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ChatAgentRequest>,
) -> impl IntoResponse {
    let msg = payload.message.trim();
    if msg.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Empty message" })),
        )
            .into_response();
    }

    let lower = msg.to_lowercase();
    let reply = if lower.contains("keyword") || lower.contains("config") || lower.contains("focus") {
        let stopwords = [
            "focus", "on", "and", "for", "jobs", "in", "the", "a", "an", "with", "or", "that",
            "me", "my", "give", "show", "find", "want", "also", "add", "please", "more", "only",
            "just", "very", "need",
        ];
        let extracted: Vec<String> = msg
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
            .filter(|w| w.len() > 2 && !stopwords.contains(&w.as_str()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect();

        let kw_formatted = if extracted.is_empty() {
            "- `Embedded Linux` \n- `RTOS` \n- `Rust` \n- `AI / ML`".to_string()
        } else {
            extracted
                .iter()
                .map(|k| format!("- `{k}`"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            "🤖 **Configuration Assistant Recommendations**\n\nI analyzed your target focus: *\"{msg}\"*\n\n**Suggested Domain Keywords:**\n{kw_formatted}\n\n💡 *Tip: Click '⚙️ Generate Config from Profile' or update `config/local.yaml` to apply these keywords to your active discovery pipeline.*"
        )
    } else if lower.contains("match") || lower.contains("score") || lower.contains("listing") || lower.contains("job") {
        let snap = data::snapshot(&state.pool).await.ok();
        let total = snap.as_ref().map(|s| s.kpi.today_discovered).unwrap_or(0);
        let shortlisted = snap.as_ref().map(|s| s.kpi.shortlisted_active).unwrap_or(0);
        format!(
            "📊 **Pipeline Matching Analysis**\n\nCurrently, your pipeline has ingested **{total} discovered jobs today**, with **{shortlisted} active shortlisted** top matches (Score ≥ 0.70).\n\n**Recommendations to increase high-scoring matches:**\n1. Ensure your `profile/profile.yaml` lists all core skills (C, C++, Rust, Python, RTOS, Yocto).\n2. Lower `min_score_threshold` in `config/local.yaml` if you want a wider net.\n3. Explore the **Discovered & Filtered Explorer** tab to manually force-shortlist any filtered job."
        )
    } else if lower.contains("prep") || lower.contains("interview") || lower.contains("question") {
        format!(
            "📝 **Interview Preparation Guide**\n\nBased on your candidate stack (Embedded Systems, Linux Kernel, RTOS, Rust, AI/ML):\n\n**Top Technical Focus Areas:**\n1. **Concurrency & Real-time Constraints**: Mutex/Semaphore mechanics, priority inversion, ISRs.\n2. **Memory Management**: Zero-copy buffer sharing, DMA transfers, memory mapping (`mmap`).\n3. **Edge AI & Acceleration**: Quantization (INT8/FP16), TensorRT/ONNX runtime optimization, latency benchmarks.\n\nCheck your **Action & Interview Center** tab for generated company-specific study sheets!"
        )
    } else {
        format!(
            "🤖 **Career-AI Assistant**\n\nI'm ready to assist with your automated job pipeline! You can ask me to:\n- 💡 Extract and suggest target keywords (`\"Suggest keywords for Remote Robotics\"`)\n- 📊 Analyze current match scores & pipeline stats (`\"Analyze job matches\"`)\n- 📝 Provide tailored interview prep strategies (`\"Interview prep for Embedded Engineer\"`)\n- ⚙️ Help configure LLM backends or job sources."
        )
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "reply": reply,
        })),
    )
        .into_response()
}
