---
name: dry-run-apply
description: Submits applications with a hard dry-run-first gate. Refuses live submission to LinkedIn or Indeed without explicit ToS-risk acknowledgement. Triggers when the user says "submit application", "apply to", or asks to send a tailored application.
---

# Dry-run apply

Owns the submission step. **The default is dry-run.** Live submission
requires an explicit, literal confirmation phrase from the user — no
paraphrase accepted.

## Trigger conditions

Activate when the user:
- Says "submit", "apply", "send the application", or names a specific
  source ("apply on LinkedIn", "send to Greenhouse").
- Has an application in state `rendered` or `prepared` and is ready to
  move it forward.

## Process

### 1. Always run dry-run first

Call `careerai_apply` MCP tool with `dry_run: true`. This:
- Walks the apply flow up to the final submit click.
- Captures a screenshot of the populated form.
- Logs a `would_submit` event with the payload that would have been
  sent.
- **Never issues a network write.**

Show the user the screenshot path and a summary of the prepared payload
(resume DOCX path, cover-letter excerpt, custom-question answers if
any). Ask them to review.

### 2. If — and only if — the user explicitly confirms, prepare for live

Live submission requires a per-source confirmation gate:

#### LinkedIn or Indeed

Auto-applying via these channels **violates platform ToS**:
- LinkedIn User Agreement section 8.2 — no automated access.
- Indeed Terms of Service — no automated submission.

Before invoking the live tool call, the user MUST type the literal
phrase:

```
I_UNDERSTAND_TOS_RISK
```

No paraphrase. No abbreviation. If they type "I understand the risk" or
"yes" or "go", that is **not** the trigger phrase — re-prompt with the
exact string. This is intentional friction.

#### ATS sources (Greenhouse, Lever, Ashby, etc.)

These submit to the platform's documented API — no ToS issue. Show:
- the destination endpoint
- a summary of the payload
- the source's submit-rate-limit budget remaining today

Then accept a plain `yes` to proceed.

### 3. Live submission

Call `careerai_apply` MCP tool with `dry_run: false`. The submitter still:
- Honors the per-source `submit_enabled` config gate (cannot be
  overridden by the user without editing config — by design).
- Acquires a `governor` rate-limit permit before any network call.
- Respects quiet-hours config.

Surface the response: status code, follow-up state (`submitted`,
`failed`), and the response body excerpt.

## Safety constraints (hard rules)

1. **Never invoke `careerai_apply` with `dry_run: false` without the
   confirmation gate above.** This is non-negotiable.
2. **Never reword the `I_UNDERSTAND_TOS_RISK` phrase.** Exact string
   only. The friction is the feature.
3. **Never bypass `submit_enabled` config.** If the user says "but I
   want to submit anyway", point them at `config/local.yaml` and stop.
4. **Never retry a failed live submission silently.** Failures go through
   the normal pipeline-state path (`failed` state, audit event).
5. **Never log secrets.** The `careerai-submit` tracing redaction filter
   strips known secret shapes; do not capture raw screenshots that show
   form fields containing API keys (cover them with the "redact" mode of
   `chromiumoxide` if implemented).

## Failure modes + recovery

- **`SubmitDisabled`** — config has `submit_enabled: false` for this
  source. Tell the user to flip it in `config/local.yaml`. Don't edit
  config files unprompted.
- **`RateLimited`** — token bucket empty. Tell the user when the next
  permit will be available; the daemon will retry on its next tick if
  enabled.
- **`SourceLoginExpired`** — refresh cookies via
  `careerai cookies refresh <provider>` (`linkedin` or `naukri`) and
  re-run.
- **`ApplicationNotReady`** — application is not in `rendered` /
  `prepared`. Run the `tailor-resume` skill first.
