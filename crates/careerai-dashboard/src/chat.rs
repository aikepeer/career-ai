//! Interactive LLM Chat Agent handler for the Career-AI Dashboard.

use std::path::Path;
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

    let root = careerai_core::paths::resolve_root_env();
    let core_cfg = crate::details::load_core_config();

    // 1. Try running through the configured live LLM backend or local agent CLI
    if let Some(agent_reply) =
        run_agent_engine(&root, core_cfg.as_ref(), msg, &payload.history).await
    {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "source": "llm_agent",
                "reply": agent_reply,
            })),
        )
            .into_response();
    }

    // 2. Comprehensive built-in CareerAI knowledge engine fallback
    let reply = answer_with_builtin_intelligence(msg, &state, &root, core_cfg.as_ref()).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "source": "builtin",
            "reply": reply,
        })),
    )
        .into_response()
}

async fn run_agent_engine(
    root: &Path,
    cfg: Option<&careerai_core::config::CoreConfig>,
    prompt: &str,
    _history: &[ChatMessage],
) -> Option<String> {
    if std::env::var_os("CAREERAI_DISABLE_AGY").is_some()
        || std::env::var_os("CAREERAI_DISABLE_SUBPROCESS").is_some()
    {
        return None;
    }

    // Try running via agy, claude, or goose CLI subprocess if available
    let bin = which::which("agy")
        .or_else(|_| which::which("claude"))
        .or_else(|_| which::which("goose"))
        .ok()
        .or_else(|| {
            std::env::var_os("HOME").and_then(|home| {
                let p = std::path::PathBuf::from(home)
                    .join(".local")
                    .join("bin")
                    .join("agy");
                if p.is_file() {
                    Some(p)
                } else {
                    None
                }
            })
        })?;

    let bin_name = bin.file_name().and_then(|n| n.to_str()).unwrap_or("agent");
    let model = cfg.map_or("zai-org-glm-52-fp8", |c| {
        if c.llm.tailor_model.is_empty() {
            "zai-org-glm-52-fp8"
        } else {
            &c.llm.tailor_model
        }
    });

    let system_context = format!(
        "You are the expert CareerAI Pipeline Agent running in {}. \
        You have complete knowledge of the CareerAI architecture (discovery, Jaccard matching, \
        local deterministic tailoring, pandoc/weasyprint rendering, and stealth application submission). \
        Answer the operator concisely with actionable guidance, CLI commands, or YAML configuration tips.\n\n\
        Operator question: {}",
        root.display(),
        prompt
    );

    let mut cmd = tokio::process::Command::new(&bin);
    if bin_name.contains("agy") {
        cmd.arg("-p")
            .arg(&system_context)
            .arg("--output-format")
            .arg("json");
    } else if bin_name.contains("claude") {
        cmd.arg("-p").arg(&system_context).arg("--model").arg(model);
    } else {
        cmd.arg("run").arg("-i").arg(&system_context);
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        cmd.current_dir(root).output(),
    )
    .await
    .ok()?
    .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if let Some(resp) = json.get("response").and_then(|r| r.as_str()) {
                    return Some(resp.to_string());
                }
                if let Some(res) = json.get("result").and_then(|r| r.as_str()) {
                    return Some(res.to_string());
                }
            }
            return Some(stdout);
        }
    }

    None
}

async fn answer_with_builtin_intelligence(
    msg: &str,
    state: &Arc<AppState>,
    root: &Path,
    core_cfg: Option<&careerai_core::config::CoreConfig>,
) -> String {
    let lower = msg.to_lowercase();

    if lower.contains("portal")
        || lower.contains("search")
        || lower.contains("company")
        || lower.contains("source")
    {
        portal_discovery_reply(&lower)
    } else if lower.contains("token")
        || lower.contains("cost")
        || lower.contains("pricing")
        || lower.contains("usage")
    {
        token_cost_reply(core_cfg)
    } else if lower.contains("keyword")
        || lower.contains("config")
        || lower.contains("domain")
        || lower.contains("focus")
    {
        keyword_recommendation_reply(msg)
    } else if lower.contains("match")
        || lower.contains("score")
        || lower.contains("shortlist")
        || lower.contains("threshold")
    {
        let snap = data::snapshot(&state.pool).await.ok();
        let total = snap.as_ref().map_or(0, |s| s.kpi.today_discovered);
        let shortlisted = snap.as_ref().map_or(0, |s| s.kpi.shortlisted_active);
        let threshold = crate::details::resolve_score_threshold(core_cfg);
        matching_analysis_reply(total, shortlisted, threshold)
    } else if lower.contains("tailor")
        || lower.contains("render")
        || lower.contains("batch")
        || lower.contains("100")
    {
        batch_tailor_reply(core_cfg)
    } else if lower.contains("interview") || lower.contains("prep") || lower.contains("question") {
        interview_prep_reply(root)
    } else if lower.contains("cli") || lower.contains("command") || lower.contains("run") {
        cli_commands_reply()
    } else {
        default_agent_reply()
    }
}

fn portal_discovery_reply(query: &str) -> String {
    let tech_slugs = if query.contains("robot") || query.contains("embedded") {
        "**Target Embedded & Robotics Portals:**\n- `greenhouse`: `appliedintuition`, `skydio`, `shieldai`, `verkada`, `commaai`, `canonical`\n- `lever`: `replicate`, `groq`, `mistralai`\n- `ashby`: `linear`, `ramp`, `perplexity`\n- `teamtailor`: `synmatchai`, `smart-eye`"
    } else {
        "**Target AI & Software Engineering Portals:**\n- `greenhouse`: `anthropic`, `openai`, `scaleai`, `canonical`, `modular`\n- `lever`: `replit`, `postman`, `replicate`, `huggingface`\n- `ashby`: `cohere`, `linear`, `cursor`, `perplexity`\n- `teamtailor`: `synmatchai`"
    };

    format!(
        "🌐 **ATS Job Portal Discovery & Recommendations**\n\nBased on your query, here are high-yield tech company slugs with active ATS job feeds:\n\n{tech_slugs}\n\n💡 *Tip: Add these company slugs under `sources.<ats>.companies` in `config/local.yaml`, then run `careerai discover` to index all active openings!*"
    )
}

fn token_cost_reply(cfg: Option<&careerai_core::config::CoreConfig>) -> String {
    let model = cfg.map_or("zai-org-glm-52-fp8", |c| &c.llm.tailor_model);
    let strategy = cfg.map_or("local", |c| &c.llm.strategy);
    format!(
        "💰 **LLM Token Usage & Cost Estimation**\n\n- **Active Tailoring Strategy:** `{strategy}`\n- **Configured Model:** `{model}`\n\n**Cost Estimation for 100 Shortlisted Jobs:**\n1. **Local Deterministic (`strategy: \"local\"`)**: **$0.00** (0 tokens, <1ms per job)\n2. **Gemini 2.0/3.7 Flash**: **~$0.04** total (~120k tokens)\n3. **Claude 3.5 Haiku**: **~$0.15** total\n4. **Claude 3.7 Sonnet**: **~$0.85** total\n\n⚡ *Recommendation: Use `strategy: \"local\"` for high-throughput zero-cost tailoring and reserved LLM queries for deep cover-letter personalization.*"
    )
}

fn keyword_recommendation_reply(msg: &str) -> String {
    let stopwords = [
        "focus", "on", "and", "for", "jobs", "in", "the", "a", "an", "with", "or", "that", "me",
        "my", "give", "show", "find", "want", "also", "add", "please", "more", "only", "just",
        "very", "need",
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
        "- `Embedded Linux` \n- `RTOS` \n- `Rust` \n- `AI / ML` \n- `Yocto`".to_string()
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
}

fn batch_tailor_reply(cfg: Option<&careerai_core::config::CoreConfig>) -> String {
    let strategy = cfg.map_or("local", |c| &c.llm.strategy);
    format!(
        "🎯 **Batch Tailoring & Rendering at Scale (100+ Jobs)**\n\n- **Current Mode:** `{strategy}`\n- **Zero-Cost Speed:** Local deterministic tailoring processes 100+ listings in **<200ms** at **$0.00** token cost.\n- **Batch Commands:**\n  - `careerai tailor <id>` (tailor single listing)\n  - `careerai run --tailor --render` (tailor and compile all shortlisted to PDF)\n  - Dashboard: Select multiple cards in Kanban and click **🎯 Tailor Selected**."
    )
}

fn cli_commands_reply() -> String {
    "⚡ **CareerAI CLI Command Guide**\n\n- `careerai discover`: Scrape all active ATS feeds and job boards.\n- `careerai match`: Score all discovered listings against candidate profile.\n- `careerai rematch`: Re-score and demote/promote listings against updated threshold.\n- `careerai tailor <id>`: Generate tailored bullet points & cover letter.\n- `careerai render <id>`: Compile tailored application to PDF + DOCX.\n- `careerai run`: Run end-to-end pipeline in one shot.\n- `careerai doctor`: Self-test database, LLM backend, pandoc, and scraper connectivity.".to_string()
}

fn interview_prep_reply(root: &Path) -> String {
    format!(
        "📝 **Interview Preparation Guide**\n\nWorkspace root: `{}`\n\n**Top Core Technical Focus Areas:**\n1. **Real-time Concurrency**: Mutex vs Semaphore, Priority Inversion, ISR handling, Lockless Ring Buffers.\n2. **Linux Kernel & Device Drivers**: MMIO, DMA transfers, Character drivers, Device Tree bindings, Interrupt handling.\n3. **Memory Optimization**: Cache locality, Zero-copy IPC, Slab allocators.\n4. **Edge AI Optimization**: INT8 Quantization, TensorRT / ONNX latency profiling.\n\nCheck your **Action & Interview Center** tab for generated role-specific interview sheets!",
        root.display()
    )
}

fn matching_analysis_reply(total: u64, shortlisted: u64, threshold: f32) -> String {
    format!(
        "📊 **Pipeline Matching Analysis**\n\nCurrently, your pipeline has ingested **{total} discovered jobs today**, with **{shortlisted} active shortlisted** top matches (Score ≥ {threshold:.3}).\n\n**Recommendations to tune your shortlist:**\n1. Use word-boundary keywords (e.g. `C`, `C++`, `RTOS`, `Linux`) in `config/local.yaml`.\n2. Adjust `match.score_threshold` (e.g. `0.025`–`0.035`) using the slider on the Config tab.\n3. Click **🔄 Rematch Shortlisted** to automatically prune low-scoring listings."
    )
}

fn default_agent_reply() -> String {
    "🤖 **CareerAI Interactive Copilot**\n\nI'm your full-system career pipeline assistant! You can ask me to:\n- 🌐 **Find new job portals & ATS slugs** (`\"Search for embedded robotics company portals\"`)\n- 💰 **Calculate token usage & batch costs** (`\"Estimate cost for 100 jobs with Gemini Flash\"`)\n- 🎯 **Tune matching & keywords** (`\"Suggest keywords for Remote Linux Kernel roles\"`)\n- 📊 **Analyze shortlisted match scores** (`\"Analyze job matches\"`)\n- ⚡ **Guide CLI commands & batch rendering** (`\"How do I tailor and render 100 jobs?\"`)\n\nWhat would you like to explore?".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_analysis_references_the_effective_threshold_key() {
        let reply = matching_analysis_reply(3, 2, 0.01);
        assert!(reply.contains("Score ≥ 0.010"), "reply was: {reply}");
        assert!(
            reply.contains("`match.score_threshold`"),
            "must point at the key the matcher reads; reply was: {reply}"
        );
        assert!(reply.contains("**3 discovered jobs today**"));
        assert!(reply.contains("**2 active shortlisted**"));
    }

    #[test]
    fn token_cost_reply_mentions_local_zero_cost() {
        let reply = token_cost_reply(None);
        assert!(reply.contains("$0.00"));
        assert!(reply.contains("Local Deterministic"));
    }

    #[test]
    fn portal_discovery_reply_lists_greenhouse_and_lever() {
        let reply = portal_discovery_reply("robotics");
        assert!(reply.contains("appliedintuition"));
        assert!(reply.contains("greenhouse"));
    }
}
