//! Error types for interview-preparation generation.

#[derive(Debug, thiserror::Error)]
pub enum PrepError {
    #[error("database: {0}")]
    Database(#[from] careerai_db::DbError),
    #[error("llm: {0}")]
    Llm(#[from] careerai_llm::LlmError),
    #[error("backend: {0}")]
    Backend(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("template: {0}")]
    Template(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("profile: {0}")]
    Profile(String),
}

pub type Result<T, E = PrepError> = std::result::Result<T, E>;
