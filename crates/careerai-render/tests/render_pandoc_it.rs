//! Optional pandoc integration test.
//!
//! Skips cleanly (with a warning) if pandoc is not on PATH. PDF engine
//! is auto-detected among `xelatex` → `weasyprint`; if neither is
//! present, the PDF assertion is skipped rather than failing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_render::config::RenderConfig;
use careerai_render::pandoc::{md_to_docx, md_to_pdf};

#[tokio::test]
async fn pandoc_subprocess_roundtrip() {
    if which::which("pandoc").is_err() {
        // eprintln! is allowed under cfg(test) per crate conventions.
        eprintln!("pandoc not on PATH — skipping render_pandoc_it");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let md_path = tmp.path().join("sample.md");
    std::fs::write(
        &md_path,
        "# Sample\n\nThis is a *test* document with a [link](https://example.com).\n",
    )
    .unwrap();

    let cfg = RenderConfig {
        timeout_seconds: 30,
        ..RenderConfig::default()
    };

    // DOCX — always expected to work when pandoc is available.
    let docx_out = tmp.path().join("sample.docx");
    md_to_docx(&md_path, &docx_out, &cfg).await.unwrap();
    let docx_meta = std::fs::metadata(&docx_out).unwrap();
    assert!(
        docx_meta.len() >= 100,
        "DOCX suspiciously small: {} bytes",
        docx_meta.len()
    );

    // PDF — requires an engine. Pick whichever is installed.
    let engine = if which::which("xelatex").is_ok() {
        Some("xelatex")
    } else if which::which("weasyprint").is_ok() {
        Some("weasyprint")
    } else {
        None
    };

    let Some(engine_name) = engine else {
        eprintln!("no PDF engine (xelatex/weasyprint) on PATH — skipping PDF assertion");
        return;
    };

    let cfg_pdf = RenderConfig {
        pdf_engine: engine_name.to_string(),
        timeout_seconds: 60,
        ..RenderConfig::default()
    };
    let pdf_out = tmp.path().join("sample.pdf");
    md_to_pdf(&md_path, &pdf_out, &cfg_pdf).await.unwrap();
    let pdf_meta = std::fs::metadata(&pdf_out).unwrap();
    assert!(
        pdf_meta.len() >= 100,
        "PDF suspiciously small: {} bytes",
        pdf_meta.len()
    );
}
