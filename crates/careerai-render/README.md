# careerai-render

Renders a tailored `ResumeView` and `CoverLetter` to DOCX + PDF
artifacts via Tera templates → Markdown → `pandoc` subprocess.

## Boundary

| Owns | Never does |
|---|---|
| `render_application(cfg, view, cover, out_dir)` entry | LLM calls |
| Tera templates (`templates/{resume,cover}.tera`) | DB queries |
| pandoc subprocess invocation + binary detection | Network |
| Artifact path layout under `data/artifacts/<application_id>/` | Diff validation |

## Runtime dependency

`pandoc` must be on PATH. Render fails with
`RenderError::PandocMissing` (mapped to CLI exit code 6) if
absent. Detected via `which::which("pandoc")` at the start of
each render.

## Output

```
data/artifacts/<application_id>/
  resume.md     intermediate Markdown (kept by default; toggle
                via `cfg.render.keep_intermediate_markdown`)
  resume.docx   pandoc → DOCX
  resume.pdf    pandoc → PDF (PDF engine from cfg.render.pdf_engine,
                default `weasyprint`)
  cover.md      cover letter Markdown
  cover.docx    pandoc → DOCX
  cover.pdf     pandoc → PDF
```

## Tests

```bash
cargo test -p careerai-render
```

Snapshot tests use `insta`. Render-artifact tests extract text
from produced DOCX/PDF via `pdf-extract` + `docx-rs` and diff
against snapshots.

## PDF engine

`weasyprint` is the default. Override via
`cfg.render.pdf_engine = "wkhtmltopdf"` etc. The engine must be
on PATH; `pandoc` will surface a clear error if not.
