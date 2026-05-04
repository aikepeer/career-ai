# Security

This document describes career-ai's security posture, known accepted
risks, and invariants that must not be broken.

## Design invariants

### Dry-run by default

`auto_submit` defaults to `false`. In dry-run mode, submitters must
never issue a network write. Per-source `submit_enabled` gates must be
honored even with `--auto-submit`. Integration tests assert both via
`wiremock` expectations and log capture.

### Constrained diff

The resume tailoring step emits a constrained JSON diff that can only
reorder or rewrite existing bullets from the profile. It cannot invent
new experience, titles, dates, or employers. Schema validation in
`crates/careerai-tailor/src/guardrails/validator.rs` rejects anything
outside that grammar.

### Proper-noun leak prevention

Proper nouns harvested for guardrail validation deliberately exclude
experience bullet bodies and project descriptions. Only summary text,
company names, titles, locations, institutions, degrees, and project
names contribute to the allowed-proper-noun set. This prevents a
tailored bullet for employer A from reusing a proper noun mentioned
only in employer B's bullet. Implementation:
`crates/careerai-tailor/src/guardrails/tokens.rs::build_token_sets`.

### PII in argv

System prompts and profile content ride on a 0o600 temp file via
`--append-system-prompt-file`, never in CLI argv. Argv is visible to
other local users via `/proc/<pid>/cmdline` or `ps -ef`.
`stub_binary_does_not_leak_profile_to_argv` enforces this.

### Credential storage

Secrets (Anthropic/OpenAI keys, LinkedIn cookies, SMTP creds) are
stored in the OS keychain via the `keyring` crate. `.env` is the
fallback for CI. `tracing` has a redaction filter; there is a
regex-based test in `careerai-submit` asserting captured events
contain no known-secret shapes.

### Rate limiting

All outbound submissions go through `governor` token-buckets defined
per source in config. Quiet hours and daily caps are enforced at the
boundary. New submitters must acquire a permit before any network call.

## Known accepted risks

### RUSTSEC-2023-0071 (rsa / Marvin attack)

`rsa` is a ghost lockfile entry pulled by `sqlx-mysql`, which we do
not compile. Only the `sqlx-sqlite` driver is enabled. `cargo tree -i
rsa` confirms it is absent from the compiled graph. No upstream fix
exists; both `cargo audit` and `cargo deny` ignore it.

### LinkedIn ToS

LinkedIn + Indeed auto-apply violates their Terms of Service. This is
a documented, accepted trade-off with mitigations: dry-run default,
rate caps via `governor`, stealth via `chromiumoxide` +
`stealth-v2.js`, per-source kill-switches in config. The
`linkedin_browser` source is disabled by default.

### Prompt injection via job descriptions

Job descriptions from external ATS feeds are passed to the LLM as part
of the tailoring prompt. A malicious JD could contain prompt-injection
text. The constrained-diff grammar limits the blast radius. The
guardrail validator rejects invented content regardless of what the
JD contains. Monitor guardrail rejection rate; a spike may indicate an
injection attempt.

### LLM provider trust

Career-ai sends rendered profile YAML (work history, education,
skills) to the configured LLM provider. The `claude` CLI backend uses
the user's own API key or subscription; profile data never transits
career-ai infrastructure. API backends bill directly to the user's
account.

## Reporting

Career-ai is a single-user local tool, not a hosted service. If you
find a security issue, open a GitHub issue or email the maintainer.
