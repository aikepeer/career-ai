//! Filesystem layout for rendered artifacts.
//!
//! One directory per application id: `<artifacts_dir>/<app_id>/`.

use std::path::PathBuf;

use crate::config::RenderConfig;

#[derive(Debug, Clone)]
pub struct ArtifactLayout {
    pub root: PathBuf,
    pub resume_md: PathBuf,
    pub resume_docx: PathBuf,
    pub resume_pdf: PathBuf,
    pub cover_md: PathBuf,
    pub cover_docx: PathBuf,
}

pub fn layout_for(cfg: &RenderConfig, app_id: &str) -> ArtifactLayout {
    let root = cfg.artifacts_dir.join(app_id);
    ArtifactLayout {
        resume_md: root.join("resume.md"),
        resume_docx: root.join("resume.docx"),
        resume_pdf: root.join("resume.pdf"),
        cover_md: root.join("cover_letter.md"),
        cover_docx: root.join("cover_letter.docx"),
        root,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn layout_derives_all_five_paths_under_root() {
        let cfg = RenderConfig {
            artifacts_dir: PathBuf::from("/tmp/artifacts"),
            ..RenderConfig::default()
        };
        let layout = layout_for(&cfg, "app-123");

        assert_eq!(layout.root, Path::new("/tmp/artifacts/app-123"));
        assert_eq!(
            layout.resume_md,
            Path::new("/tmp/artifacts/app-123/resume.md")
        );
        assert_eq!(
            layout.resume_docx,
            Path::new("/tmp/artifacts/app-123/resume.docx")
        );
        assert_eq!(
            layout.resume_pdf,
            Path::new("/tmp/artifacts/app-123/resume.pdf")
        );
        assert_eq!(
            layout.cover_md,
            Path::new("/tmp/artifacts/app-123/cover_letter.md")
        );
        assert_eq!(
            layout.cover_docx,
            Path::new("/tmp/artifacts/app-123/cover_letter.docx")
        );
    }
}
