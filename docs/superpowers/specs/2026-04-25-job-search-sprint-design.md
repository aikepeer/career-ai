# Job-search sprint — design spec

**Date:** 2026-04-25
**Author:** kk + Claude
**Status:** brainstorm → design (this doc) → impl plan (next)

## Goal

Maximize **interviews scheduled per week** for the operator (kk) over the next 6 weeks, with **minimum daily engagement** (≤ 30 min/day average), so they have cognitive bandwidth left to revise concepts and prep for those interviews.

## Non-goals (explicitly deferred)

- OSS framework cleanup (adapter SDK, CONTRIBUTING.md, plugin docs)
- SaaS / multi-tenancy / hosted product
- Switching browser stack (chromiumoxide → Playwright/puppeteer-extra)
- Mutation testing, fuzz, criterion benchmarks, codecov dashboards
- M6.1 polish backlog (O(N²) match loop, `pub`→`pub(crate)`, `SchedulerError #[non_exhaustive]`, etc.)
- Indeed submitter, Glassdoor, AngelList/Wellfound, etc.

These come back on the table **after** the operator has signed an offer. Not before.

## Stage gate

**Re-evaluate this spec when ANY of these triggers fire:**
- Operator accepts a job offer → pivot to OSS framework spec
- 6 weeks elapsed with < 3 interviews scheduled → re-examine matcher quality and submit channel mix
- LinkedIn account gets restricted → kill the LinkedIn submitter immediately, lean harder on ATS HTTP
- Naukri account gets restricted → same

## Strategic framing

The current pipeline (M3–M6) can: **discover → match → tailor → render → submit (dry-run)**.

The blocker for interviews is the submit step going live. Three submit channels exist; their risk/throughput profile is asymmetric:

| Channel | ToS status | Throughput ceiling | Detection risk | Account-loss cost |
|---|---|---|---|---|
| ATS HTTP (Greenhouse, Lever, Ashby) | **Officially supported APIs** | High — limited only by job availability | None | None |
| Naukri | Tolerated; no public anti-automation stance | Medium | Low | Low (no recruiter inbound for kk) |
| LinkedIn Easy Apply | **Violates ToS** | High | Medium-high at any sustained volume | **Catastrophic during job search** — locks recruiter inbound |

**The good jobs route through ATS APIs anyway.** Most LinkedIn "Apply" buttons that lead to a real opportunity link out to the company's Greenhouse/Lever board. LinkedIn is best treated as a *discovery* surface, not the apply surface.

Therefore:

1. **ATS HTTP submitters are the workhorse.** Maximize throughput here.
2. **Naukri full auto-submit** is acceptable.
3. **LinkedIn = assist mode, not auto.** Daemon prepares the application (form-filled, screenshot taken), human clicks Submit during a daily 5-min review session. Same effective throughput, fraction of the account risk. Toggle to full auto exists but stays off during the search.

## Work items

### W1 — ATS submitters live mode + matcher tightening (week 1)

**What:** Verify Greenhouse/Lever/Ashby submitters work end-to-end against three real jobs. Add a `must_include_skills` hard filter to `MatchConfig` so junk applications stop firing.

**Why:** ATS is the highest-leverage channel and it's already implemented (M4). The matcher today is Jaccard over a small skill set; without a hard filter, low-fit jobs slip through and waste tailoring/render budget. Top failure mode in week 1 will be operator manually skipping junk in `careerai review`.

**Scope:**
- 3 live applications via each of {Greenhouse, Lever, Ashby} with `auto_submit=true` for those sources, observing `submitted_at` events and the remote application IDs.
- New config field `match.must_include_skills: Vec<String>` (default empty for backward compat). When non-empty, listings missing ALL of these skills move directly to `filtered_out`.
- Daily-digest CLI (W3) shows the filter's hit rate so the operator can tune.

**DoD:** Three real jobs submitted, three remote IDs captured, zero junk in the `tailored` queue after 48h of daemon runtime.

### W2 — M5b assist mode + M5c Naukri click (week 2)

**What:** Implement LinkedIn Easy Apply as **draft + screenshot, no submit**. Implement Naukri click as **full auto-submit**. Add a `careerai review` CLI subcommand that walks pending LinkedIn drafts and accepts Y/N/skip per draft, doing the actual click.

**Why:** Splits the LinkedIn flow into the dangerous part (clicking Submit) and the safe part (everything else), and keeps the dangerous part human-gated. Naukri's lower risk profile means full auto is fine there.

**Scope:**
- `careerai-submit::linkedin`: extend the existing dry-run path so it walks the form, screenshots, and persists `application.state = "drafted"` (new state variant — db migration adds it between `prepared` and `submitted`). Submit-click code path stays gated behind both `allow_submit_click` AND a new `interactive_only` flag (**default `true`** — daemon never clicks autonomously; only `careerai review` does, by passing `interactive_only=false` for that single call). Operator can flip the daemon-side default to `false` later if they decide full auto is worth the risk; default ships safe.
- `careerai-submit::naukri`: implement the actual form-fill + click using the Naukri Apply button selectors. Same governor limiter pattern as LinkedIn.
- `careerai-cli`: new `review` subcommand. Loads `application.state = "drafted"` LinkedIn rows, displays JD summary + screenshot path + "Submit? [y/N/s(kip)]". On `y`, calls into `careerai-submit::linkedin::confirm_submit(application_id)` which performs the click.
- Cookie-refresh helper: `careerai cookies refresh` walks the operator through grabbing a fresh `li_at` from a real browser session.

**DoD:**
- LinkedIn: 5 drafts produced, 5 reviews completed via `careerai review`, 5 successful submits with remote IDs.
- Naukri: 3 live submits with no human intervention.
- New ListingState `drafted` between `prepared` and `submitted` for the LinkedIn assist path.

### W3 — daily-digest + observability (end of week 2)

**What:** `careerai digest` prints yesterday's pipeline state — discovered count, matched count, drafts pending, submitted count, responded count — plus per-source filter hit rates and any error blips.

**Why:** Without a daily summary the operator has no idea if the daemon is working. Five lines of structured output replaces 20 minutes of manual SQL.

**Scope:**
- New CLI subcommand `digest [--since 24h]`. Reads from existing `events` table.
- One row per pipeline stage, colored if non-zero failures.
- Dump path for any application that errored, so the operator can re-run manually.

**DoD:** Run `careerai digest` after a daemon shift, get a useful report in < 1 second.

### W4 — interview prep bolt-on (week 3)

**What:** New crate `careerai-prep`. When an application is marked `responded` (recruiter replied, not auto-rejection), the prep crate fetches the JD + company info + the resume diff that was sent and writes a study sheet to `prep/<application_id>.md`.

**Why:** This is the closing-the-loop bit. If the daemon is working, recruiter replies start landing. The operator should not have to manually rebuild context for each interview — the daemon already has all the inputs.

**Scope:**
- New crate `careerai-prep` depending on `careerai-tailor` (for the diff), `careerai-llm` (for the study sheet generation), `careerai-render`.
- `careerai-prep::generate(application_id)` produces `prep/<id>.md` with sections: JD summary, likely topics from JD keywords, behavioral question shortlist, the resume bullet → JD-keyword map for talking points, company recent news (LLM-summarized).
- **Trigger:** the `responded` state transition is operator-driven for now (`careerai mark-responded <application_id> --note "recruiter replied"`). Auto-detection from inbox is explicitly out of scope for this sprint — Gmail/IMAP poll + classifier is its own milestone (M7 inbox watcher) and would expand W4 to two weeks. The `mark-responded` CLI flips the state and the existing scheduler tick (or a one-shot `careerai prep <id>` invocation) fires the prep generator.
- LLM-generated content goes through the same prompt-cache + constrained-output discipline the tailor crate uses (no inventing facts; ground all claims in the JD or company web search results).

**DoD:** First operator-marked `responded` application produces a `prep/<id>.md` within 60 seconds; operator reads it, no manual context-rebuild needed.

### W5–W6 — operate

No new code. Daily 5-min review of LinkedIn drafts via `careerai review`. Weekly 30-min config tune based on what the digest shows. Interviews land on the calendar.

If interview count is low after week 5: re-evaluate matcher + filter + JD volume per source. Consider widening the must-include-skills filter or adding more Greenhouse companies.

## Risks + mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| LinkedIn account restricted during search | Medium with full auto, low with assist mode | Assist mode (W2). Submit cron remains dry-run by default. `interactive_only=true` flag layered on top of existing two locks. |
| Naukri rate-limit / soft-block | Low | Conservative governor settings (max 5/day, 30s+ between, jitter). Same pattern as M5a. |
| Tailor over-aggressive → invented experience on resume | Critical | Existing constrained-diff validator (M3 invariant) blocks this. Don't relax it. Adversarial test stays in CI. |
| Operator forgets to refresh `li_at` cookie | High | `careerai cookies refresh` helper. Daemon logs `WARN cookie expires in 2 days` based on JWT exp claim. |
| Daemon crashes silently overnight | Medium | systemd unit with `Restart=on-failure`. `careerai digest` shows last-tick timestamp; if > 2h, alarm. |
| Resume PII leak in logs | Low | Existing redaction filter in `careerai-submit`. Add a quick regex test asserting the events table never contains raw email addresses or phone numbers. |
| Operator's own interview prep gets neglected because daemon is "working on it" | High (this is the point of the project) | Daily digest forces them to see what's pending. `prep/` directory is the explicit interview-prep workspace. Calendar block 2h/day for actual studying — outside this codebase. |

## Out-of-scope-for-now backlog (parked until job offer signed)

- M6.1: O(N²) match-loop → HashMap; `pub`→`pub(crate)` on pipeline internals; `SchedulerError #[non_exhaustive]`; cron-expr serde validation; H6 panic-injection IT; `run_until_shutdown` SIGINT test; T1+T2 docstring tightening from H6+H7 review.
- OSS prep: CONTRIBUTING.md, GitHub Actions matrix, dependabot, codecov, public README rewrite, sample fixtures, semantic-versioning + changelog.
- Testing rigor: cargo-mutants, proptest on the constrained-diff validator + state machine, criterion benchmarks, cargo-fuzz on YAML/HTML/JSON parsers.
- Browser stack: evaluate puppeteer-extra-stealth-style plugin equivalents in Rust (or Node sidecar) to harden against future LinkedIn detection upgrades.
- Indeed, AngelList/Wellfound, Glassdoor, Hacker News Who-is-hiring submitters.
- Multi-channel outreach: cold email to hiring managers, referral-finder via LinkedIn graph, follow-up email after N days of no response.
- Dashboard / web UI for non-CLI users.

These are NOT bad ideas. They are *future* ideas. Current state is "operator needs a job"; current state is not "operator has time to implement features for hypothetical OSS users".

## Operator-side runbook (preview — full version ships with W3)

**One-time setup (~30 min):**

1. `cargo install --path crates/careerai-cli` to put `careerai` on PATH.
2. `careerai init` to scaffold `$XDG_CONFIG_HOME/career-ai` and `$XDG_DATA_HOME/career-ai` (using standard per-user defaults when unset).
3. `cp tests/fixtures/profile.yaml $XDG_DATA_HOME/career-ai/profile/profile.yaml`, edit with real info.
4. `careerai cookies refresh` to capture `li_at` into the OS keychain. Walk-through prompt.
5. Set `ANTHROPIC_API_KEY` via `keyring` or env.
6. Edit `$XDG_CONFIG_HOME/career-ai/config/local.yaml`: enable Greenhouse companies you target, set `must_include_skills`, set `submit.auto_submit=true`, set `submit.per_source.{greenhouse,lever,ashby,naukri}.enabled=true`. Leave `submit.per_source.linkedin.enabled=false` initially OR `submit.linkedin.interactive_only=true` (default).
7. `careerai daemon` in a tmux session OR `systemctl --user enable --now careerai.service`.

**Daily (~5 min):**

```
careerai digest
careerai review            # walks LinkedIn drafts, y/N/s
```

**Weekly (~15 min):**

- Skim `events.csv` export of last 7 days. Tune `must_include_skills`, `score_threshold`, source list in `local.yaml`.
- Refresh `li_at` if WARN line appears in `careerai digest`.

**Kill switch:**

```
systemctl --user stop careerai.service       # or Ctrl-C the tmux daemon
careerai inspect <application_id>            # audit trail per application
careerai daemon-cleanup                      # cancel any in-flight cron, drain queues
```

If LinkedIn flags the account: stop the daemon, set `submit.per_source.linkedin.enabled=false`, lean on ATS + Naukri until the account recovers (recovery typically 24–72h). Do not retry LinkedIn submits during a flag; that compounds the problem.

## Success metric

**Primary:** number of interviews scheduled per calendar week, weeks 4–6.
**Secondary:** ratio of (recruiter replies / submitted applications), tells us if the matcher is targeting the right jobs.
**Failure metric:** number of accounts restricted. Target: 0.

## Next step

Once this spec is approved, invoke `superpowers:writing-plans` to expand W1–W4 into a step-by-step implementation plan. W5–W6 are operate-mode and don't need a plan.
