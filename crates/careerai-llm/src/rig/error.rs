//! Retry classification and error-mapping helpers.

use crate::error::LlmError;

/// Retry classifier. Transient network/timeout/rate-limit errors retry;
/// schema errors and hard upstream failures (4xx other than 429) do not.
pub(crate) fn is_retryable(err: &LlmError) -> bool {
    matches!(err, LlmError::RateLimited { .. } | LlmError::Timeout { .. })
        || matches!(err, LlmError::Upstream(msg) if is_transient_upstream(msg))
}

pub(crate) fn is_transient_upstream(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("timed out")
        || m.contains("timeout")
        || m.contains("connection reset")
        || m.contains("connection closed")
        || m.contains("broken pipe")
        || m.contains("dns")
        || m.contains("502")
        || m.contains("503")
        || m.contains("504")
}

// Takes `reqwest::Error` by value because it's used with `.map_err(...)` and
// that closure API moves the error.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn reqwest_to_llm_err(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout { seconds: 0 }
    } else {
        LlmError::Upstream(e.to_string())
    }
}
