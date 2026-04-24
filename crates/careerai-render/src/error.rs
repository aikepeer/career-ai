//! Error type for the render crate.

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tera: {0}")]
    Tera(#[from] tera::Error),
    #[error(
        "pandoc not found on PATH; install via `apt install pandoc` / `brew install pandoc` \
         (see README). Override via render.pandoc_bin in config."
    )]
    PandocMissing,
    #[error("pandoc failed (exit {code}): {stderr_tail}")]
    Pandoc { code: i32, stderr_tail: String },
    #[error("pandoc timed out after {seconds}s")]
    TimedOut { seconds: u64 },
    #[error("which: {0}")]
    Which(#[from] which::Error),
}

pub type Result<T> = std::result::Result<T, RenderError>;
