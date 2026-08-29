//! Anthropic API key lookup. Mirrors `careerai-cli::anthropic_key_reachable`
//! so behavior is consistent across CLI and library callers.

const ENV_KEY_NAMES: &[&str] = &[
    "LLM_API_KEY",
    "CAREERAI_LLM_API_KEY",
    "DEEPSEEK_API_KEY",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "GROK_API_KEY",
    "XAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "CLAUDE_WEB_API_KEY",
];

#[cfg(feature = "live-llm-api")]
pub(super) fn read_api_key() -> Option<String> {
    for &var_name in ENV_KEY_NAMES {
        if let Ok(v) = std::env::var(var_name) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    keyring::Entry::new("career-ai", "anthropic/api_key")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|v| !v.is_empty())
}

#[cfg(feature = "live-llm-api")]
pub(super) fn detect_provider() -> crate::rig::Provider {
    if std::env::var("DEEPSEEK_API_KEY").is_ok_and(|v| !v.is_empty())
        || std::env::var("DEEPSEEK_API_BASE_URL").is_ok_and(|v| !v.is_empty())
        || std::env::var("OPENAI_API_KEY").is_ok_and(|v| !v.is_empty())
        || std::env::var("OPENAI_API_BASE_URL").is_ok_and(|v| !v.is_empty())
        || std::env::var("OPENROUTER_API_KEY").is_ok_and(|v| !v.is_empty())
        || std::env::var("GROK_API_KEY").is_ok_and(|v| !v.is_empty())
        || std::env::var("XAI_API_KEY").is_ok_and(|v| !v.is_empty())
        || std::env::var("OLLAMA_API_BASE_URL").is_ok_and(|v| !v.is_empty())
    {
        return crate::rig::Provider::OpenAI;
    }
    if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return crate::rig::Provider::Anthropic;
    }
    if let Some(key) = read_api_key() {
        if key.starts_with("sk-ant-") {
            return crate::rig::Provider::Anthropic;
        }
    }
    crate::rig::Provider::OpenAI
}

pub(super) fn api_key_source() -> Option<&'static str> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:ANTHROPIC_API_KEY");
    }
    if std::env::var("DEEPSEEK_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:DEEPSEEK_API_KEY");
    }
    if std::env::var("OPENAI_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:OPENAI_API_KEY");
    }
    if std::env::var("GROK_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:GROK_API_KEY");
    }
    if std::env::var("XAI_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:XAI_API_KEY");
    }
    if std::env::var("OPENROUTER_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:OPENROUTER_API_KEY");
    }
    if std::env::var("CAREERAI_LLM_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:CAREERAI_LLM_API_KEY");
    }
    if std::env::var("CLAUDE_WEB_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Some("env:CLAUDE_WEB_API_KEY");
    }
    let entry = keyring::Entry::new("career-ai", "anthropic/api_key").ok()?;
    let v = entry.get_password().ok()?;
    if v.is_empty() {
        None
    } else {
        Some("keyring:career-ai/anthropic/api_key")
    }
}
