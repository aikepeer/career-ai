//! Render configuration: DOCX/PDF artifact generation via pandoc.
//!
//! `RenderConfig` is an additive field on [`CoreConfig`] (`#[serde(default)]`).
//! It is re-exported from `careerai_core::config` and also re-exported by
//! `careerai_render` so the render crate can be used without reaching across
//! crates for the type.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    pub artifacts_dir: std::path::PathBuf,
    pub pandoc_bin: Option<std::path::PathBuf>,
    pub pdf_engine: String,
    pub timeout_seconds: u64,
    pub keep_intermediate_markdown: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            artifacts_dir: std::path::PathBuf::from("artifacts"),
            pandoc_bin: None,
            pdf_engine: "weasyprint".into(),
            timeout_seconds: 60,
            keep_intermediate_markdown: true,
        }
    }
}
