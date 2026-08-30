//! ATS text-layer verification against a real pandoc-generated PDF.
//!
//! Skips cleanly (with a warning) if pandoc is not on PATH; requires a
//! PDF engine exactly like `render_pandoc_it.rs` (xelatex → weasyprint).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_render::ats::{extract_jd_keywords, verify_pdf};
use careerai_render::config::RenderConfig;
use careerai_render::pandoc::md_to_pdf;

#[tokio::test]
async fn verify_pdf_finds_contact_and_keywords_in_rendered_resume() {
    if which::which("pandoc").is_err() {
        eprintln!("pandoc not on PATH — skipping ats_pdf_it");
        return;
    }
    let engine = if which::which("xelatex").is_ok() {
        "xelatex"
    } else if which::which("weasyprint").is_ok() {
        "weasyprint"
    } else {
        eprintln!("no PDF engine on PATH — skipping ats_pdf_it");
        return;
    };

    let tmp = tempfile::tempdir().unwrap();
    let md = tmp.path().join("resume.md");
    std::fs::write(
        &md,
        "# Alice Kumar\n\
         Senior ML Engineer\n\
         alice.kumar@example.com  |  +91 98765 43210\n\
         \n\
         Built RAG pipelines for LLM applications at Acme Robotics.\n\
         Rust, Python, Kubernetes, ROS2.\n",
    )
    .unwrap();
    let pdf = tmp.path().join("resume.pdf");
    let cfg = RenderConfig {
        pdf_engine: engine.to_string(),
        timeout_seconds: 90,
        ..RenderConfig::default()
    };
    md_to_pdf(&md, &pdf, &cfg).await.unwrap();

    // The JD side: keywords derived the same way the pipeline does it.
    let jd = "Senior ML Engineer at Acme Robotics — build RAG pipelines \
              with Kubernetes, Rust and Python for LLM applications.";
    let keywords = extract_jd_keywords(jd, 8);

    let report = verify_pdf(
        &pdf,
        "alice.kumar@example.com",
        "+91-98765-43210",
        &keywords,
    )
    .expect("verify rendered PDF");

    assert!(
        report.garbage.is_empty(),
        "clean template must not emit garbage markers: {:?}",
        report.garbage
    );
    assert!(
        report.contact.iter().all(|c| c.present),
        "contact must survive as literal text: {:?}",
        report.contact
    );
    for hit in &report.keywords {
        assert!(
            hit.present,
            "keyword {:?} should be extractable from the rendered PDF (extracted {} chars)",
            hit.keyword, report.text_len
        );
    }
}
