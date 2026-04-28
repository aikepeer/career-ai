---
name: dry-run-apply
description: "Submits applications with a hard dry-run-first gate. Refuses live submission to LinkedIn or Indeed without explicit ToS-risk acknowledgement, and without a per-source `submit.per_source.<source>.enabled: true` flip in `config/local.yaml`. Triggers when the user says 'submit application', 'apply to', or asks to send a tailored application."
---

# Dry-run apply

Owns the submission step. **The default is dry-run.** Live submission
requires (a) an explicit, literal confirmation phrase from the user,
and (b) a deliberate per-source `submit.per_source.<source>.enabled:
true` flip in `config/local.yaml` — the user does that themselves;
you do not edit their config.

## Trigger conditions

Activate when the user:

- Says "submit", "apply", "send the application", or names a specific
  source ("apply on LinkedIn", "send to Greenhouse").
- Has an application in state `rendered` or `prepared` and is ready to
  move it forward.

## Process

### 1. Always run dry-run first

Call `careerai_apply` MCP tool with `dry_run: true`. This:

- Calls each `Submitter::prepare()` to build the `would_submit`
  envelope. The browser session is **not** launched — dry-run today
  does not produce a screenshot for browser-driven submitters
  (LinkedIn / Naukri). Screenshots only land on the live path (and
  the LinkedIn assist/review flow).
- Logs a `would_submit` event with the prepared payload (resume DOCX
  path, cover-letter excerpt, custom-question answers).
- **Never issues a network write.**

Show the user:

- The submitter that would have fired (greenhouse / lever / linkedin /
  naukri / ...).
- A summary of the `would_submit` payload — endpoint or DOM target,
  resume path, cover-letter excerpt.

Ask them to review.

### 2. If — and only if — the user wants to go live, walk them through the flips

Live submission requires three things in this order:

#### 2a. ToS-risk acknowledgement (LinkedIn / Indeed only)

Auto-applying via these channels **violates platform ToS**:

- LinkedIn User Agreement §8.2 — no automated access.
- Indeed Terms of Service — no automated submission.

The user MUST type the literal phrase:

```
I_UNDERSTAND_TOS_RISK
```

No paraphrase. No abbreviation. If they type "I understand the risk"
or "yes" or "go", that is **not** the trigger phrase — re-prompt with
the exact string. This is intentional friction.

For ATS sources (Greenhouse, Lever, Ashby, etc.) there is no ToS
issue. Show the destination endpoint, a summary of the payload, and
the source's submit-rate-limit budget remaining today. A plain `yes`
is enough.

#### 2b. `submit.per_source.<source>.enabled: true` for that ONE source

Per-source `enabled` gates default to `false` and are honored even
when the user passes `--auto-submit`. **You do not edit
`config/local.yaml` yourself.** Print the YAML they should add:

```yaml
# config/local.yaml — flip ONE source at a time
submit:
  per_source:
    <source-name>:
      enabled: true
```

Stop and wait for them to save the file. Then proceed.

#### 2c. The live call

Call `careerai_apply` MCP tool with `dry_run: false`. The submitter
still:

- Re-checks the per-source `submit.per_source.<source>.enabled`
  config gate (the user could have flipped it back; defense in depth).
- Acquires a `governor` rate-limit permit before any network call.
- Respects quiet-hours config.

Surface the response: status code, follow-up state (`submitted`,
`failed`, `skipped`), and the response body excerpt.

### 3. Recap and follow-up

After a live submission:

- Run `careerai applied --limit 20` to confirm the row is recorded.
- Run `careerai digest --since 24h` if the user wants the whole-pipeline
  view (counts by state, per-source breakdown, cookie expiry).

## Safety constraints (hard rules)

1. **Never invoke `careerai_apply` with `dry_run: false` without
   completing steps 2a + 2b above.** This is non-negotiable.
2. **Never reword the `I_UNDERSTAND_TOS_RISK` phrase.** Exact string
   only. The friction is the feature.
3. **Never edit `config/local.yaml` for the user.** Print the YAML
   they should add and wait. Editing config silently bypasses the
   deliberate per-source decision.
4. **Never bypass `submit.per_source.<source>.enabled`.** If the user
   says "but I want to submit anyway", point them at
   `config/local.yaml` and stop.
5. **Never retry a failed live submission silently.** Failures go
   through the normal pipeline-state path (`failed` state, audit
   event).
6. **Never log secrets.** The `careerai-submit` tracing redaction
   filter strips known secret shapes; do not capture raw screenshots
   that show form fields containing API keys.

## Failure modes + recovery

Outcomes / errors come from
`crates/careerai-submit/src/base.rs::SubmitOutcome` and
`crates/careerai-submit/src/error.rs::SubmitError`:

- **`SubmitOutcome::Skipped { reason: "source disabled" }`** —
  `submit.per_source.<source>.enabled` is `false` (the default) for
  that source in `config/local.yaml`. The application + listing
  transition to `skipped`; there is no retry. Tell the user to flip
  the gate (per the YAML in step 2b). Don't edit the file.
- **`SubmitError::SourceDisabled(...)`** — used for *runtime* policy
  blocks (rate-limit denied, quiet-hours window, missing credentials,
  or a browser submitter like LinkedIn that intentionally aborts at
  the pre-submit gate such as `allow_submit_click`). Use the message
  text to distinguish the cases; if it is user-configurable, tell the
  user what they need to change.
- **`SubmitError::BadState { state }`** — application is not in
  `rendered` / `prepared` / `drafted`. Run the `tailor-resume` skill
  first; the error names the actual state.
- **`SubmitError::NotImplemented(...)`** — submitter exists in the
  registry but its click/network flow has not been built. Distinct
  from `SourceDisabled` so the audit log can tell "engineer hasn't
  shipped this yet" from "operator turned this off / a runtime gate
  fired". Tell the user the source is on the roadmap.
- **`SubmitError::HttpStatus { status, body_tail }`** — the ATS
  rejected the submission. Surface the status + tail to the user; the
  audit row has the full response.
- **Login / session expired at the source** (`linkedin` / `naukri`) —
  refresh cookies via `careerai cookies refresh <provider>` and re-run
  from step 1.
- **Rate-limit condition** — the token bucket is empty. Tell the user
  when the next permit will be available; the daemon will retry on
  its next tick if enabled.
