---
description: Submit a prepared application. Defaults to dry-run; live submission requires confirmation, a per-source config flip, and a ToS-risk acknowledgement.
argument-hint: "<app-id> [--auto-submit]"
---

# /career:apply

Submits a single application that is already in state `rendered` or
`prepared`. **Defaults to dry-run.** A dry-run never issues a network
write — it calls `Submitter::prepare()` to build a `would_submit`
envelope, logs the event, and exits. Browser-driven submitters
(LinkedIn / Naukri) are **not** launched under dry-run today, so no
screenshot is produced from this path.

This page is a step-by-step walkthrough. Follow the steps in order;
do not skip ahead to `--auto-submit` without doing the dry-run review
first.

## Arguments

- `<app-id>` (required): UUID of the application to submit. Get one
  from `/career:status` or `careerai applied --limit 20`.
- `--auto-submit` (optional): force live submission. Only valid after
  steps 1–4 below.

---

## Step 1 — Run the dry-run (no flag needed)

```
/career:apply <app-id>
```

This is always safe. The submitter:

- Resolves the listing's source (greenhouse, lever, ashby, linkedin,
  naukri, indeed, ...).
- Calls `Submitter::prepare()` to build a `would_submit` envelope
  (resume DOCX path, cover-letter excerpt, custom-question answers,
  endpoint / DOM target). The dry-run path stops here — it does
  **not** call `Submitter::submit()`, so browser-driven submitters
  (LinkedIn / Naukri) do not launch a session and do not capture a
  screenshot. Screenshots are only produced on the live path (and
  the LinkedIn assist/review flow).
- Logs the `would_submit` event.
- Exits without sending anything.

---

## Step 2 — Review the dry-run outcome

The output names the submitter that fired and the `would_submit`
record. For each source, expect to see:

| Source kind | What to verify |
|---|---|
| ATS HTTP (greenhouse / lever / ashby / ...) | Endpoint URL, request body keys (resume, cover_letter, profile fields, custom answers), per-source rate-limit budget remaining today. |
| Browser (linkedin, naukri) | The `would_submit` payload only — DOM target URL, resume path, cover-letter excerpt. Dry-run does not launch a browser session today, so no screenshot is captured here. |
| Email (when wired) | Recipient address, subject, body excerpt, attachment paths. |

Drill into the audit row if anything looks off:

```
careerai inspect <app-id>
```

Stop here if the payload is wrong — fix the underlying data and re-run
`/career:tailor <listing-id>` before continuing.

---

## Step 3 — Confirm the ToS-risk recap (browser sources only)

Auto-applying via **LinkedIn** or **Indeed** violates their Terms of
Service:

- LinkedIn User Agreement §8.2 — no automated access.
- Indeed Terms of Service — no automated submission.

If your application's source is one of these, you must explicitly type
the literal phrase below before live submission:

```
I_UNDERSTAND_TOS_RISK
```

No paraphrase. No abbreviation. The friction is the feature.

For ATS sources (greenhouse / lever / ashby / smartrecruiters / ...) a
plain `yes` is enough, since you're posting to the platform's
documented public API.

---

## Step 4 — Flip `submit.per_source.<source>.enabled: true` for that ONE source

Per-source `enabled` gates default to `false` and **are honored even
with `--auto-submit`**. The flag is intentional: live-submit must be
a deliberate, per-source operator decision.

I will **not** edit your config for you. Open `config/local.yaml` in
your editor and add (or merge into your existing `submit:` block) the
single source you want to enable:

```yaml
# config/local.yaml — DO NOT enable more than the source you are about
# to submit to. Each source needs its own deliberate flip.
submit:
  per_source:
    greenhouse:
      enabled: true
    # leave others off — `enabled: false` (the default) is correct
```

Save the file. The next `/career:apply` invocation will re-read
config; no daemon restart needed.

If you forget this step, the live submit is treated as
`SubmitOutcome::Skipped { reason: "source disabled" }`, transitions
the application/listing to `skipped`, and **does not retry**.

---

## Step 5 — Re-run with `--auto-submit` for live submission

```
/career:apply <app-id> --auto-submit
```

Even with `--auto-submit`:

- The per-source `submit.per_source.<source>.enabled` gate from
  step 4 is still required.
- The call goes through `governor` token-buckets. If the bucket is
  empty or you're inside a quiet-hours window, the call is blocked at
  the rate-limit boundary and is **not** retried silently. The error
  surfaces with the next-permit time.
- Failures land in pipeline state `failed` with a row in `events` —
  inspect via `careerai inspect <app-id>`.

Live output prints: source, submission timestamp, response code, and
the new follow-up state.

---

## Step 6 — Recap exit conditions and check follow-ups

A live submission ends in one of these terminal states:

| State | Meaning | Next step |
|---|---|---|
| `submitted` | The submitter accepted the request and you got a non-error response. | Wait for response tracking (M7); for now, watch your inbox. |
| `failed` | The submitter raised an error. Today this includes rate-limit and quiet-hours blocks, since `run_live` treats `SubmitError::SourceDisabled` from those gates as a failure. | `careerai inspect <app-id>` to read the error variant; back off and retry once a token is available or the quiet-hours window closes. |
| `skipped` | Per-source `submit.per_source.<source>.enabled` is `false`, or the source has no submitter wired (feed-only / `--features browser` not built in). The submit layer returns `SubmitOutcome::Skipped` before any network call. | The audit log names the reason. Flip the gate or rebuild with the right feature; no retry happens automatically. |
| `responded` | A response landed (M7, planned). | Not yet wired — see the README "Known gaps" section. |

To see what you've submitted recently and at what cadence (the CLI
does not currently support a `--since` window — use `--limit` and/or
`--source`):

```
careerai applied --limit 20
```

To roll up the whole pipeline (counts by state, per-source breakdown,
last cron tick, cookie expiry warnings):

```
careerai digest --since 24h
```

---

## Tool call

Calls the `careerai_apply` MCP tool with:

- `application_id: <app-id>`
- `dry_run: true` (default) or `false` (only after steps 1–4 above)

## Failure modes (quick reference)

- `SubmitError::BadState` — application is not in `rendered` or
  `prepared`. Run `/career:tailor <listing-id>` first.
- `SubmitOutcome::Skipped { reason: "source disabled" }` —
  `submit.per_source.<source>.enabled` is `false` (the default) for
  that source in `config/local.yaml`. Application/listing land in
  `skipped`. See step 4.
- `SubmitError::SourceDisabled(...)` — runtime policy block:
  rate-limit denied, quiet-hours window, missing credentials, or a
  browser submitter aborting at its pre-submit gate (e.g. LinkedIn's
  `allow_submit_click=false`). `run_live` currently treats this as a
  failure, so the application transitions to `failed`. Back off and
  retry once the gate clears.
- Login / session expired (cookie sources) — refresh via
  `careerai cookies refresh <provider>` (`linkedin` and `naukri` are
  the supported providers) and re-run from step 1.
