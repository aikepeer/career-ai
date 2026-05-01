//! Anthropic API key lookup. Mirrors `careerai-cli::anthropic_key_reachable`
//! so behavior is consistent across CLI and library callers.

#[cfg(feature = "live-llm-api")]
pub(super) fn read_api_key() -> Option<String> {
    if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
        if !v.is_empty() {
            return Some(v);
        }
    }
    keyring::Entry::new("career-ai", "anthropic/api_key")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|v| !v.is_empty())
}

pub(super) fn api_key_source() -> Option<&'static str> {
    if std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty())
    {
        return Some("env:ANTHROPIC_API_KEY");
    }
    let entry = keyring::Entry::new("career-ai", "anthropic/api_key").ok()?;
    let v = entry.get_password().ok()?;
    if v.is_empty() {
        None
    } else {
        Some("keyring:career-ai/anthropic/api_key")
    }
}
