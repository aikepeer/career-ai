---
description: Submit a prepared application. Defaults to dry-run; --live requires explicit ToS-risk confirmation.
argument-hint: "<app-id> [--live]"
---

# /career:apply

Submits a single application that is already in state `rendered` or
`prepared`. **Defaults to dry-run.** A dry-run never issues a network write —
it produces a screenshot and logs a `would_submit` event, then exits.

## Arguments

- `<app-id>` (required): UUID of the application to submit.
- `--live` (optional): force live submission. **See safety section below.**

## Safety — read this carefully

1. **Default is dry-run.** Calling this command with no flags is always safe.
   The submitter walks through the apply flow up to the final click, captures
   a screenshot, logs `would_submit`, and stops.
2. **`--live` requires confirmation.** When `--live` is passed, the model
   MUST:
   - Identify the listing's source (greenhouse, lever, ashby, linkedin,
     indeed, ...).
   - For LinkedIn or Indeed: state explicitly that auto-applying via these
     channels violates the platform's ToS, then require the user to type
     the literal phrase `I_UNDERSTAND_TOS_RISK` before invoking the tool.
     No paraphrase, no abbreviation.
   - For ATS sources (greenhouse / lever / ashby / etc.): state that the
     submission will hit the ATS's public API endpoint, summarize what will
     be sent (resume, cover letter, profile fields, source-specific custom
     answers), and require a plain `yes` confirmation.
3. **Per-source `submit_enabled` gates are honored even with `--live`.** If
   the source has `submit_enabled: false` in config, the call fails fast
   with `SubmitError::SourceDisabled(...)` — `--live` does not override
   config.
4. **Rate limits + quiet hours apply.** Even with `--live`, the call goes
   through `governor` token-buckets. If the bucket is empty or you're in a
   quiet-hours window, the call is blocked at the rate-limit boundary and
   is not retried silently. (Rate-limit/quiet-hours conditions are also
   surfaced through the same disabled-source error path with a message.)

## Tool call

Calls the `careerai_apply` MCP tool with:
- `application_id: <app-id>`
- `dry_run: true` (default) or `false` (only after confirmation as above)

## Output

Dry-run prints: source, would-submit log line, screenshot path.
Live prints: source, submission timestamp, response code, follow-up state.

## Failure modes

- `SubmitError::BadState` — the application is not in `rendered` or
  `prepared`. Run `/career:tailor` first.
- Rate limiting or quiet-hours gating — back off; submission is blocked
  until tokens are available or the quiet-hours window has passed. The
  daemon retries on its next tick.
- Login/session issues for cookie-based sources may surface as credential,
  cookie, or keyring-related errors. Refresh cookies via
  `careerai cookies refresh <provider>` (`linkedin` and `naukri` are the
  supported providers) and re-run.
