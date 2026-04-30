# Security

career-ai is a **single-user, local-first** tool. There is no public
service, no shared backend, and no upstream that holds your data. The
threat model is the operator's own laptop and the third-party services
they reach out to (job boards, ATSs, LLM providers, notification
endpoints). This document captures the posture, the safety invariants
the codebase enforces, and how to report a vulnerability.

If you are reading this because a dependency CVE was filed against
career-ai, see the **Advisories** section below for the current
posture.

## Reporting a vulnerability

Please email **capitalbluecity@gmail.com** with `[career-ai security]`
in the subject. Use [GitHub Security
Advisories](https://github.com/justdoGIT/career-ai/security/advisories/new)
for coordinated disclosure if the issue affects users beyond your own
laptop. Do not file public issues for security reports.

Reasonable timeline: acknowledgement within 72 hours, fix or
mitigation within 14 days for High/Critical, longer for hardening
items. There is no bug bounty.

## Threat model

| Threat | In scope | Out of scope |
|---|---|---|
| Untrusted operator on the same laptop | yes (file perms + keyring) | not multi-user |
| Network MITM against ATS / LLM endpoints | yes (TLS, rustls) | corporate MITM proxies you've voluntarily installed |
| Malicious profile or LinkedIn export | yes (parsers fail closed; no eval) | a profile YAML the operator hand-wrote with bad intent |
| Compromised LLM output | yes (constrained-diff guardrails) | LLM jailbreaks against the operator's own prompt |
| LinkedIn / job-board ToS enforcement | partially (mitigations below) | the legal exposure of automated apply itself |
| Compromised crates.io dep (supply chain) | yes (cargo-deny + cargo-audit) | a typosquat the operator manually adds to Cargo.toml |
| The user's own laptop being stolen | no | use full-disk encryption |

The product is **not** designed for: hosted/SaaS deployment, processing
job listings or profiles for someone other than the operator, exposing
the daemon's HTTP surface beyond loopback, or running with elevated
privileges. Each of these would re-open threats this tool consciously
ignores.

## Safety invariants enforced by the codebase

These are the load-bearing properties. Each one has tests asserting
the invariant; if you touch the related modules, keep the tests
strict.

### 1. Constrained-diff resume tailoring

The LLM may **only reorder or reword existing bullets** from the
operator's profile. It cannot invent new experience, employers,
titles, dates, projects, or numbers. Schema is in
`crates/careerai-tailor/src/diff.rs`; the validator rules are in
`crates/careerai-tailor/src/guardrails.rs`.

The validator rejects rewords that introduce:

- Proper nouns not present in the profile summary, structured
  identifier fields (company / project / institution / title /
  location), per-bullet original text, the curated common-English
  whitelist, or the skill list.
- Years not present in the profile.
- Numbers not present in the profile.
- Inflated counts of any of the above.

`forbid_invented_entities` carries the spec; `forbid_invented_entities_with`
is the per-call hot path that pre-computes token sets across all
rewords in a tailor session.

The 2026-04 Codex review flagged a "cross-bullet token leak" — a
proper noun mentioned only in employer B's bullet could leak into
employer A's tailoring. Closed in PR #53; see commit `9c4e1b6`.

### 2. Dry-run-by-default for submit

`auto_submit` defaults to `false`. In dry-run mode, submitters must
**never issue a network write** — they screenshot + log a
`would_submit` event. Per-source `submit_enabled` gates are honored
even with `--auto-submit`. Tests in `careerai-submit` assert both via
`wiremock` expectations and log capture.

If you add a new submitter, the burden is on you to keep the dry-run
path free of any HTTP/RPC writes. Audit by running
`cargo test -p careerai-submit -- --nocapture` and grepping the
captured wiremock state.

### 3. Rate limiting at the boundary

All outbound submissions go through `governor` token-buckets defined
per source in config. Quiet hours and daily caps are enforced at the
boundary, not in the submitter. New submitters must acquire a permit
before any network call. There is no override flag.

### 4. Loopback-only HTTP surface

The dashboard server (`careerai status serve`) binds to
`127.0.0.1:8787` by default. The `--bind` flag is hidden; non-loopback
binds emit a loud no-auth warning to stderr because **there is no
authentication**. Future work for a real domain will add bearer-token
auth from the keyring; until then, do not expose the dashboard beyond
the loopback interface.

### 5. Atomic file writes for config

`careerai sources sync --apply` and `careerai service install` use
`tempfile::NamedTempFile::persist` for atomic file replacement. This
gives `rename(2)` semantics on Unix and
`MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows so SIGKILL or
power loss between write and persist leaves the original file intact.
The Windows variant matters because bare `std::fs::rename` rejects an
existing destination on Windows (closed in PR #52).

## Credentials & secret handling

- **OS keychain via the `keyring` crate** is the primary store for
  LinkedIn cookies, Anthropic / OpenAI API keys, and SMTP
  credentials. Read at startup; never logged.
- **`.env` files** are the fallback for CI-like environments. They
  are excluded from `git status -uall` via `.gitignore`. Do not commit
  `.env` files.
- **Notification webhook secrets** (Slack, Telegram, ntfy) come from
  environment variables or the keyring. Inline secrets in YAML are
  intentionally rejected with a parse error in `careerai-notify`.
- **`tracing` redaction**: a regex-based test in `careerai-submit`
  asserts captured tracing events contain no known-secret shapes
  (cookie values, bearer tokens, API keys). If you add a new sensitive
  log field, extend the redaction regex and the test.

If a secret leaks into a log line during development, treat it as a
production incident: rotate the credential, then check
`.remember/logs/` for any file that captured it.

## LinkedIn / Indeed ToS posture

LinkedIn and Indeed auto-apply violate their respective Terms of
Service. The project ships with this fully understood and these
mitigations:

- **Default off**: `sources.linkedin_browser.enabled = false`,
  `submit.linkedin.submit_enabled = false`. The operator must flip
  these explicitly.
- **Dry-run default**: even with sources enabled, the apply path
  defaults to `would_submit` events.
- **Per-source rate caps**: `governor` buckets enforce both per-tick
  and per-day limits.
- **Stealth**: `chromiumoxide` runs with a checked-in `stealth-v2.js`
  pinned by SHA. When Chrome CDP changes break it, update the script
  and its hash in the same commit as the selector fixes.
- **Quiet hours**: a configurable window that pauses all submissions.

Treat the legal exposure as the operator's responsibility. The project
does not provide legal advice. If you operate in a jurisdiction where
ToS violations carry liability beyond account suspension (e.g. CFAA
exposure in the United States), do not use the auto-apply paths.

## Supply-chain policy

### Advisory & license gates

- **`cargo audit`** runs in CI on every PR
  (`.github/workflows/ci.yml::audit`). The single ignored advisory is
  documented inline in that job.
- **`cargo deny check`** runs in CI per `deny.toml` at the repo root.
  Policy: error on advisories, allow only the standard permissive
  license set (MIT / Apache-2.0 / BSD / ISC / MPL-2.0 / BSL-1.0 /
  Unicode-3.0 / CDLA-Permissive-2.0), deny unknown registries and
  unknown git remotes.
- **`semgrep`**: per-project baseline at
  `.claude/.semgrep-baseline.json` per the global rule. Re-scan after
  PR merge with ≥ 20 changed files, dep-manifest bumps, or any change
  touching auth / crypto / SQL / deserialization.

### Pinned-version policy

- Workspace-level dependencies are pinned via `Cargo.lock` (committed).
- Wildcard versions are denied by `cargo-deny` (`bans.wildcards = "deny"`).
- The `stealth-v2.js` browser script is pinned by SHA in
  `careerai-sources/src/linkedin_browser*.rs`.

### Cross-platform CI

- **`windows-check`** job in CI cross-compiles the workspace against
  `x86_64-pc-windows-gnu` from a Linux runner with `mingw-w64`. This
  exists because two Windows-incompat regressions (PR #44 with
  `tokio::signal::unix`, PR #52 with bare `std::fs::rename`) shipped
  and were caught only after release.

## Advisories

### RUSTSEC-2023-0071 (rsa Marvin attack)

Status: **ignored, with rationale**.

`rsa` is a transitive dep of `sqlx-mysql`, which we never compile.
Only `sqlx-sqlite` is enabled in our feature set. `cargo tree -i rsa`
confirms `rsa` is not reachable in the compiled graph for any binary
this workspace ships. No upstream fix exists (the `rsa` crate has not
issued a constant-time-comparison patch); the path forward depends on
`sqlx` pruning the unused dep.

The ignore is consistent across both `cargo audit` (workflow) and
`cargo deny` (`deny.toml::advisories.ignore`). Re-evaluate when:
- `sqlx` releases a version that drops the `rsa` dep in
  `sqlx-mysql`, or
- we add a feature that pulls in `sqlx-mysql` (in which case the
  advisory becomes load-bearing and the ignore must be removed).

## Security checklist for contributors

If your change touches any of the following, bring extra eyes:

- [ ] LLM input or output handling (`careerai-llm`, `careerai-tailor`)
- [ ] Submit path (`careerai-submit`) — both dry-run and live
- [ ] Browser sources (`careerai-sources/src/linkedin_browser*`)
- [ ] Config parsing (`careerai-core/src/config.rs`) — anything that
      receives operator-supplied paths or URLs
- [ ] HTTP surface of the dashboard (`careerai-dashboard`)
- [ ] Cookie / credential handling (`careerai-submit/src/credentials.rs`)
- [ ] DB migrations that touch persisted secrets

For all of the above, the PR template should include:
- Which safety invariant from this doc is exercised.
- Whether the change requires a re-baselined `semgrep` scan.
- Whether the change needs a new test asserting the invariant.
