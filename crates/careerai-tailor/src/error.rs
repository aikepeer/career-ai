//! Error types for the tailor crate.
//!
//! `Result<T>` aliases to `Result<T, TailorError>`. Only `thiserror` here —
//! no `anyhow` in library crates. Every variant carries enough context for
//! operators to act on it from the log line alone.

#[derive(Debug, thiserror::Error)]
pub enum TailorError {
    #[error("llm: {0}")]
    Llm(#[from] careerai_llm::LlmError),

    #[error("db: {0}")]
    Db(#[from] careerai_db::error::DbError),

    #[error("profile: {0}")]
    Profile(String),

    #[error("schema: {0}")]
    Schema(String),

    // `original_bullet` is kept as a struct field for tests + downstream
    // inspectors but the Display form truncates it to keep logs tight and
    // avoid leaking long bullet text into tracing output where it may be
    // shipped to aggregators.
    #[error(
        "invented content at {path}: reason={reason}; offending_token={offending_token:?}; \
         original_bullet={}",
        truncate_for_log(.original_bullet, 64)
    )]
    InventedContent {
        path: String,
        offending_token: String,
        original_bullet: String,
        reason: &'static str,
    },

    #[error("cover letter too long: {words} words (cap {cap})")]
    CoverLetterTooLong { words: usize, cap: usize },

    #[error("cover letter too long: {chars} chars (cap {cap})")]
    CoverLetterCharsTooLong { chars: usize, cap: usize },

    #[error("bad bullet path: {0}")]
    BadPath(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("cache key: {0}")]
    CacheKey(String),

    #[error("tera: {0}")]
    Tera(String),

    #[error("yaml: {0}")]
    Yaml(String),
}

pub type Result<T> = std::result::Result<T, TailorError>;

/// Truncate a string for log display. Keeps the first `max` chars and
/// appends a length suffix when truncation occurred. Referenced by
/// `TailorError::InventedContent`'s Display via `thiserror`.
pub(crate) fn truncate_for_log(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        format!("{s:?}")
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head:?}...[truncated, {n} total chars]")
    }
}

impl From<tera::Error> for TailorError {
    fn from(e: tera::Error) -> Self {
        Self::Tera(e.to_string())
    }
}

impl From<serde_yaml::Error> for TailorError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::Yaml(e.to_string())
    }
}
