use thiserror::Error;

#[derive(Debug, Error)]
pub enum MatchError {
    #[error("config: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, MatchError>;
