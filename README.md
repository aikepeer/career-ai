# career-ai

Automated job discovery, resume tailoring, and auto-apply for a single user.

**Status:** M0 (workspace scaffold) and M1 (profile ingestion) merged.
M2 (discovery + matching) in review. See `CLAUDE.md` for development
guidance and the approved milestone plan.

## What it does (target v1)

Finds jobs across ATS APIs (Greenhouse, Lever, Ashby), remote aggregators
(Remotive, We Work Remotely, RemoteOK), Wellfound/YC, LinkedIn, and Indeed,
matches them against your profile, tailors a resume and cover letter per job
description, and submits applications with a dry-run safety gate by default.

Niche focus: AI/ML + LLM apps, and embedded platforms / robotics. Remote-first
with Delhi-NCR fallback.

## Build

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Quick start

```bash
cargo run -p careerai-cli -- init           # scaffolds config/, profile/, .env
cargo run -p careerai-cli -- profile import \
    ~/Downloads/resume.pdf \
    ~/Downloads/LinkedIn-Export.zip          # PDF/DOCX/LinkedIn-zip → profile.yaml
cargo run -p careerai-cli -- profile validate
cargo run -p careerai-cli -- --help
```

`profile import` accepts any combination of `.pdf`, `.docx`, and a
LinkedIn data-export `.zip`; the canonical merge writes
`profile/profile.yaml`. `profile show` and `profile validate` round-trip
that file against the schema. Pass `--force` to overwrite an existing
profile.

## Runtime dependencies

- `pandoc` on PATH (used at M3 for DOCX + PDF rendering).
- OS keychain (Linux Secret Service, macOS Keychain) for credentials.

## Safety + legal

LinkedIn and Indeed auto-apply violates their Terms of Service. This tool
ships dry-run by default, enforces per-source rate caps, uses stealth
browser techniques, and requires explicit opt-in per source before real
submissions. See `CLAUDE.md` for the full risk register.
