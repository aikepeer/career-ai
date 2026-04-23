//! PDF → plain text via `pdf-extract`.
//!
//! Heavy layout (two-column, embedded images) will flatten poorly — that's
//! expected at M1. The LLM fallback described in the plan will land in M3
//! once `careerai-llm` is wired up.

use std::path::Path;

use crate::error::{ProfileError, Result};

/// Extract plain text from a PDF file on disk.
pub fn extract_text_from_path(path: &Path) -> Result<String> {
    pdf_extract::extract_text(path).map_err(|e| ProfileError::Pdf(e.to_string()))
}

/// Extract plain text from in-memory PDF bytes (used in tests).
pub fn extract_text_from_bytes(bytes: &[u8]) -> Result<String> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| ProfileError::Pdf(e.to_string()))
}
