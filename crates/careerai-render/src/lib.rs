//! Rendering to DOCX + PDF.
//!
//! Templating via `tera` → Markdown → `pandoc` subprocess. Single-column,
//! ATS-safe output. Pandoc is an external runtime dependency. Implemented
//! in M3.
