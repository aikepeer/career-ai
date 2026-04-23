# career-ai

Automated job discovery, resume tailoring, and auto-apply for a single user.

**Status:** pre-implementation. See `/home/kk/.claude/plans/federated-riding-mochi.md`
for the approved plan and `CLAUDE.md` for development guidance.

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

## Quick start (post-M0)

```bash
cargo run -p careerai-cli -- init           # scaffolds config/, profile/, .env
$EDITOR profile/profile.yaml                 # seed your profile
cargo run -p careerai-cli -- --help
```

## Runtime dependencies

- `pandoc` on PATH (used at M3 for DOCX + PDF rendering).
- OS keychain (Linux Secret Service, macOS Keychain) for credentials.

## Safety + legal

LinkedIn and Indeed auto-apply violates their Terms of Service. This tool
ships dry-run by default, enforces per-source rate caps, uses stealth
browser techniques, and requires explicit opt-in per source before real
submissions. See `CLAUDE.md` for the full risk register.
