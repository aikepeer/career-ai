# CareerAI: feature plan and code review

Date: 2026-09-09  
Status: planning and review only; proposed fixes and features are not implemented by this document.  
Baseline: current working tree, including pre-existing uncommitted work, on top of commit `5fca282`.

## Objective and scope

Continue the direction in [the product review](PRODUCT_REVIEW_AND_PLAN.md): help a candidate find credible matches, prepare accurate applications, explicitly approve submission, and track outcomes. Complete these journeys before expanding the number of loosely connected tools.

This document reviews submission, state transitions, tailoring, content reuse, dashboard error handling, follow-ups, analytics, and their database boundaries. It records the issues found in those areas and proposes new features with acceptance criteria. It is not an exhaustive audit of every source adapter, dependency, deployment, or security boundary. Code-inspection findings are distinguished from missing product capabilities; the reproduction cases below are proposed regression tests, not claims of completed runtime verification.

## Existing foundation to extend

The working tree already contains:

- A dashboard workspace with saved searches, a command palette, activity summaries, explorer filters, and application details.
- Discovery adapters, matching, local and LLM tailoring, cover-letter skeletons, bullet-variant compilation, rendering, and submission gates.
- Storage and initial surfaces for follow-ups, referrals, interview feedback, quality scores, salary observations, and application variants.
- Resume guardrails, an artifact download registry, same-origin checks, optional dashboard token authentication, and a service worker restricted to static assets.

These are implementation foundations, not a declaration that every workflow is complete. In particular, the presence of a module or database table does not establish that its data reaches the main pipeline correctly.

## Review priorities

- **P0:** address before expanding unattended submission or automatic reuse of application content.
- **P1:** address during the next reliability delivery; these affect correctness, recovery, or user trust.
- **P2:** address as the associated feature is completed or scaled.

Historical findings are tracked below; current status is recorded in the
reconciliation sections and each remaining item stays explicitly labeled.

| ID | Priority | Finding | Main area |
| --- | --- | --- | --- |
| R01 | P0 | Concurrent submission and ambiguous remote success can produce duplicates | Submit / database |
| R02 | P0 | Reused cover letters can retain another company's or profile's content | Tailor / content library |
| R03 | P1 | LinkedIn assist can move completed applications back to drafted | Pipeline |
| R04 | P1 | Preparation failures leave application, listing, and retry history inconsistent | Pipeline / database |
| R05 | P1 | Live mode can override the configured local tailoring strategy | Pipeline |
| R06 | P1 | Dashboard timeouts do not explicitly terminate and reap commands | Dashboard |
| R07 | P1 | Tailor and render persistence can leave partially completed stages | Pipeline / database |
| R08 | P1 | Rollback conflicts with new foreign keys and permits forward state changes | Pipeline / migrations |
| R09 | P1 | Follow-up creation reports success when insertion fails | Scheduler |
| R10 | P1 | Follow-up scheduling can duplicate drafts or associate them with the wrong attempt | Scheduler / database |
| R11 | P1 | Operational failures and skips are counted as employer responses or rejections | Analytics |
| R12 | P1 | Database failures appear as empty results or missing applications | Dashboard |
| R13 | P1 | Shortlist validation and its state update can race | Dashboard / database |
| R14 | P2 | Application transitions do not verify the supplied listing belongs to the application | Database |
| R15 | P2 | Audit ordering is unstable when timestamps tie | Database |
| R16 | P2 | A/B tracking lacks actual treatment identity and complete outcome data | Submit / analytics |
| R17 | P2 | Explorer silently limits the searchable dataset | Dashboard |
| R18 | P2 | Follow-up cards omit company/title context and lack a complete handling workflow | Dashboard |

## Findings and proposed fixes

### R01 — submission needs a durable claim and recovery record

**Evidence:** [submit_application and run_live](../crates/careerai-submit/src/submit.rs) read an eligible state, perform the external submission, and persist success afterward. There is no shared claim covering all submission entry points. [LinkedIn's claim](../crates/careerai-db/src/queries/linkedin.rs) changes `drafted` to `rendered`, which remains eligible for batch application. The dashboard's process-local mutex does not coordinate with another CLI process or the daemon.

**Trigger and impact:** two workers can read the same eligible application before either writes success. A remote submission can also succeed before a local database write fails, leaving a retry capable of sending it again.

**Proposed fix:** introduce a persisted submission-attempt record and an atomic claim that excludes claimed work from ordinary queues. Record approval identity, attempt ID, remote receipt when available, and timestamps. Represent uncertain remote results explicitly; reconcile them before retrying. Use provider idempotency keys where supported. Scope LinkedIn review claims to LinkedIn applications and define recovery after a crash.

**Acceptance:** concurrent CLI/daemon attempts produce one mock remote submission; losing workers do not change the winner's state. A simulated crash or database failure after remote success enters a recoverable uncertain state. Do not promise exactly-once remote delivery where the provider cannot support it.

### R02 — content-library reuse must preserve recipient and candidate identity

**Evidence:** [draft_cover_letter_smart](../crates/careerai-tailor/src/lib.rs) falls back to a stored complete letter when the generated letter is shorter than 40 characters. [fetch_cover_letter_for_domain](../crates/careerai-db/src/queries/content_library.rs) selects by domain and role only. The stored text is returned unchanged, without checking the current company or profile hash.

**Trigger and impact:** create a letter for Company A, then tailor a similar role at Company B with a short generated cover letter. The fallback can address Company A or repeat facts from an earlier profile.

**Proposed fix:** reuse validated, recipient-neutral skeletons with explicit slots. Key candidate-specific material by profile version, retain provenance, fill the current company/title, and validate the final letter. Treat a database error separately from a cache miss.

**Acceptance:** two-company and changed-profile fixtures cannot carry old names, contact details, or unsupported claims into a new letter. Missing or invalid reusable content takes an explicit fallback path.

### R03 — LinkedIn assist bypasses the ordinary state guard

**Evidence:** [apply_one](../crates/careerai-pipeline/src/apply.rs) enters the `interactive_only` branch before calling the submit layer's state validation and unconditionally transitions both rows to `drafted`.

**Trigger and impact:** applying an already submitted, responded, failed, or unprepared LinkedIn application can reopen it for review. Reapplying an existing draft also adds a redundant transition event.

**Proposed fix:** validate application and listing state before drafting, make repeated drafting idempotent, and enforce the expected states in the same transaction as the write. Preserve existing assist behavior for eligible applications.

**Acceptance:** terminal and unprepared states remain unchanged; repeated drafting creates one meaningful event; a concurrent completed submission cannot be moved backward by a stale drafting request.

### R04 — failures must preserve a coherent retry state

**Evidence:** [apply_all_detailed](../crates/careerai-pipeline/src/apply.rs) calls `set_application_state("failed")` for any error. Preparation failures before `run_live` do not receive the submit layer's paired listing transition. `retry_application` reads the last listing failure event and defaults to `rendered` if none exists.

**Trigger and impact:** a missing payload or invalid profile can leave an application failed while its listing remains prepared/rendered. Retrying a prepared application can restore the wrong state. An unconditional batch failure write can also overwrite a row advanced by another worker.

**Proposed fix:** centralize failure recording for single and batch application paths; atomically update eligible rows and append an attempt-specific event. Preserve the original error and pre-submit state. Distinguish transient policy deferrals, preparation failures, uncertain remote results, and stale-state conflicts.

**Acceptance:** missing-payload and invalid-profile cases keep both rows consistent; retry restores the recorded state; repeated failures do not duplicate the same event; stale batch errors cannot overwrite a submitted row.

### R05 — local strategy must remain local when live mode is enabled

**Evidence:** [tailor_one_with_pool](../crates/careerai-pipeline/src/tailor.rs) recognizes `cfg.llm.strategy == "local"` inside a `!is_live` condition. [Dashboard command execution](../crates/careerai-dashboard/src/profile_handler/pipeline.rs) sets `CAREERAI_LLM_LIVE=1` for its subprocesses.

**Trigger and impact:** a dashboard user configures local tailoring but, without the separate strategy environment override, the pipeline can resolve a live backend and send profile/JD content to it.

**Proposed fix:** make the selected strategy authoritative. Resolve configuration and environment precedence in a pure helper before constructing a backend, and reject invalid strategy values.

**Acceptance:** local mode performs zero LLM calls across live/offline flags and CLI/dashboard entry points. Test hybrid threshold boundaries, missing scores, and explicit overrides separately.

### R06 — timeout handling needs process cleanup

**Evidence:** [run_subprocess](../crates/careerai-dashboard/src/profile_handler/pipeline.rs) wraps `Command::output()` in a timeout without explicit child termination or reaping. After timeout, the request returns and its busy guard is released.

**Trigger and impact:** a long-running command can continue after the dashboard reports failure, allowing a second request to overlap it.

**Proposed fix:** retain ownership of the child, terminate it on timeout/cancellation, and await cleanup before releasing the command lock. Define descendant cleanup for LLM/browser subprocesses and bound captured output. Killing a command must not be interpreted as proof that a remote submission did not occur; integrate with R01.

**Acceptance:** a controlled long-running child and its descendants stop after timeout, the endpoint returns a structured timeout, and the next command starts only after cleanup. Verify Unix and Windows behavior separately.

### R07 — stage persistence needs transaction boundaries

**Evidence:** [render_one](../crates/careerai-pipeline/src/render.rs) attaches artifacts, transitions the listing, and updates the application in separate database operations. [LLM tailoring](../crates/careerai-tailor/src/lib.rs) and [local tailoring](../crates/careerai-tailor/src/local.rs) separately create an application, write its payload, and advance the listing.

**Trigger and impact:** a write failure can leave an application without its payload, or a listing rendered while its application remains tailored. A retry may create another incomplete attempt.

**Proposed fix:** persist the core application/payload/state/event changes together. Stage rendered files first, then atomically register the complete artifact set and stage transition; define cleanup for unregistered files. Keep optional analytics writes outside the core success contract.

**Acceptance:** injected failures at each database write either retain the previous complete stage or produce the next complete stage. Retrying does not duplicate application attempts or artifact registrations.

### R08 — rollback needs explicit semantics and dependent-data handling

**Evidence:** [rollback_one_with_pool](../crates/careerai-pipeline/src/rollback.rs) deletes applications after deleting only artifacts and payloads. [Follow-ups](../crates/careerai-db/migrations/0006_follow_ups.sql), [variants](../crates/careerai-db/migrations/0007_ab_variants.sql), and [interview feedback](../crates/careerai-db/migrations/0008_interview_feedback.sql) reference applications without delete cascades. The target accepts any parseable listing state.

**Trigger and impact:** rollback to discovered/shortlisted can fail on a foreign-key constraint once these records exist. A target such as `submitted` can also create a submission state/event without an actual submission.

**Proposed fix:** define allowed backward transitions, preserve historical submitted attempts, and explicitly archive, detach, or remove dependent records according to their meaning. Put manual outcome recording in its own operation. Avoid blanket cascading deletion of useful history.

**Acceptance:** rollback handles applications with follow-ups, variants, and feedback atomically; invalid forward targets are rejected; historical submission evidence remains available.

### R09 — follow-up insert failures are counted as success

**Evidence:** [check_and_create_follow_ups](../crates/careerai-scheduler/src/follow_ups.rs) discards `create_follow_up` errors and increments `created` unconditionally.

**Trigger and impact:** an insert failure produces a success count and log message even though no draft exists.

**Proposed fix:** propagate insertion errors or return a structured partial-success report. Increment created only after a successful insert and surface the failure through the dashboard check endpoint.

**Acceptance:** a database trigger rejecting an insert produces a visible failure and no created count; after repair, retry creates the missing draft once.

### R10 — follow-up scheduling needs attempt identity and cadence rules

**Evidence:** [the scheduler](../crates/careerai-scheduler/src/follow_ups.rs) chooses the latest application for a listing without checking whether that application was submitted, and checks for a pending follow-up before a separate insert. [The table](../crates/careerai-db/migrations/0006_follow_ups.sql) has no uniqueness rule for the logical follow-up. Only pending entries suppress another draft.

**Trigger and impact:** concurrent scheduler/manual checks can create duplicates; an older submitted attempt can cause a draft for a newer unsubmitted application; marking a draft sent makes another eligible on the next check regardless of the intended follow-up cadence.

**Proposed fix:** associate each follow-up with a real submitted attempt and cadence step. Use an atomic insert with a corresponding uniqueness constraint. Define how sent, snoozed, dismissed, and responded states affect the next reminder.

**Acceptance:** concurrent checks create one draft, later unsubmitted applications receive none, and handled reminders recur only when their next configured step is due. Draft creation never sends email.

### R11 — operational state is not an employer outcome

**Evidence:** [funnel_velocity](../crates/careerai-db/src/queries/patterns.rs) includes skipped/failed rows in submitted and responded counts, and labels them rejected. `rejection_latencies` interprets skipped/failed events as employer rejection events.

**Trigger and impact:** a disabled-source skip or submission error can be reported as an employer response. Failure-before-submission histories can yield meaningless or negative rejection latency.

**Proposed fix:** separate operational pipeline status from employer outcomes. Calculate reached stages from real stage events, and rejection latency only from a submission and subsequent explicitly recorded rejection for the same attempt.

**Acceptance:** dry runs, source skips, rendering failures, and network errors create no employer response or rejection. Submitted-then-rejected fixtures produce the expected positive latency and sample count.

### R12 — preserve the difference between unavailable and empty

**Evidence:** [api_analytics and index loading](../crates/careerai-dashboard/src/handlers/mod.rs) turn several failed queries into empty/default values. [fetch_application_detail](../crates/careerai-dashboard/src/details/application.rs) treats general lookup errors as missing records and drops payload/artifact/timeline errors.

**Trigger and impact:** a database outage looks like an empty analytics page, a missing application, or an application with no artifacts. Users lose the reason and a useful retry path.

**Proposed fix:** map only `NotFound` to absence. Return failures or explicit per-section unavailable states for other errors, preserve previous client results, and provide retry controls. Keep detailed diagnostics in logs without exposing sensitive payloads.

**Acceptance:** closed-pool and malformed-query fixtures produce unavailable/5xx responses rather than empty success or 404. Genuine empty datasets still render an ordinary empty state.

### R13 — shortlist validation must be atomic with the write

**Evidence:** [api_force_shortlist](../crates/careerai-dashboard/src/handlers/listings.rs) reads and checks a listing, then calls [transition](../crates/careerai-db/src/queries/listings.rs), whose update has no expected-state condition.

**Trigger and impact:** another worker can advance the listing after the check, and the stale request can move it back to shortlisted or add a duplicate event.

**Proposed fix:** use a conditional transition from discovered/filtered-out in a transaction; return conflict when the expected state no longer matches. Preserve the idempotent response for an already-shortlisted row.

**Acceptance:** a deterministic interleaving of shortlist and pipeline advancement cannot regress the later state or duplicate the successful shortlist event.

### R14 — verify application/listing association in shared transitions

**Evidence:** [transition_application_and_listing](../crates/careerai-db/src/queries/applications_sync.rs) accepts two IDs but updates the application by its ID alone and independently updates the supplied listing. This is an invariant gap in the helper; no mismatched production caller was established in this review.

**Proposed fix and acceptance:** constrain the write to the application's actual listing. A test pairing application A with listing B must fail without changing either row or adding an event; normal paired transitions must remain atomic.

### R15 — give audit events a deterministic tie-breaker

**Evidence:** [events_for and list_recent_events](../crates/careerai-db/src/queries/events.rs) order by `created_at` alone. Multiple transitions can share a stored timestamp.

**Proposed fix and acceptance:** add event ID as a secondary ordering key in the same direction. Equal-timestamp fixtures must preserve forward transition order and stable reverse pagination. Audit related timestamp comparisons for mixed timestamp formats when updating these queries.

### R16 — A/B results need real variant provenance

**Evidence:** [run_live](../crates/careerai-submit/src/submit.rs) assigns A/B from application-ID bytes after submission, without linking the label to a distinct resume treatment. [record_variant](../crates/careerai-db/src/queries/variants.rs) does not populate `submitted_at`; the response updater has no production caller found in the reviewed tree.

**Impact:** these rows cannot establish that one resume style caused better outcomes, and their submission/outcome metadata is incomplete.

**Proposed fix and acceptance:** record the actual content version and treatment assignment before review, then attach verified submission time and manually recorded outcomes. Prevent duplicate records for one assignment. Display sample sizes and incomplete observations; avoid declaring a winner from arbitrary labels or tiny samples.

### R17 — explorer search does not cover every stored listing

**Evidence:** [api_explorer](../crates/careerai-dashboard/src/handlers/mod.rs) requests at most 10,000 listings, while [the explorer count](../crates/careerai-dashboard/src/details/explorer.rs) counts the full table. Client-side filtering searches the returned subset.

**Proposed fix and acceptance:** add server-side filters and stable pagination, returning total and matching counts. A search fixture with more than 10,000 rows must find an older matching listing and distinguish loaded rows from all results.

### R18 — follow-up cards need actionable context

**Evidence:** [api_follow_ups](../crates/careerai-dashboard/src/handlers/mod.rs) fills company and title with empty strings and exposes only a 200-character body preview. [Routes](../crates/careerai-dashboard/src/routes.rs) provide listing/checking but no complete edit/snooze/handled flow.

**Proposed fix and acceptance:** join application/listing context and add the full reminder lifecycle described in F05. Each card must identify its job, open the full draft, and support a persisted user decision without implying that a message was sent.

## Verification status — 2026-09-22

The findings below remain the historical review record. Their current
disposition is:

- **Verified in the current tree:** R03 LinkedIn assist guards; R05 local
  tailoring strategy; R06 subprocess cleanup; R09 follow-up insert errors;
  R10 follow-up attempt/cadence uniqueness; R11 analytics separation; R12
  database errors are propagated; R13 conditional shortlist transitions; R14
  application/listing association checks; R15 deterministic audit ordering;
  R17 server-side explorer filtering/pagination; and R18 follow-up listing
  context and handling lifecycle.
- **Implemented but not fully closed:** R01 has durable submission-attempt
  records and claim/recovery paths, but provider idempotency and uncertain
  remote-result reconciliation still depend on each adapter. R02 has
  profile/content identity protections, but every recipient-validation case
  still needs coverage. R04, R07, and R08 have failure, transaction, and
  rollback handling, but historical remote-side effects cannot be erased.
  R16 records content provenance, while statistically meaningful experiment
  outcomes still require real observations.
- **Still roadmap work:** the F01–F10 feature rows below are proposals unless
  the current code and an acceptance check explicitly say otherwise.

Verification evidence for this reconciliation includes the dashboard library
suite, four serial real-browser dashboard tests, scheduler follow-up regression
tests, and the XDG workspace smoke check. It does not claim live ATS/provider
submissions or privileged service installation.
The full release workspace suite passed with
`cargo test --release --workspace --all-targets --locked --no-fail-fast`.


## New feature roadmap

These are proposed user-facing increments. Existing modules should be reused where their contracts are sound.

| ID | Priority | Feature and user value | Proposed scope | Acceptance criteria | Dependencies |
| --- | --- | --- | --- | --- | --- |
| F01 | P1 | Guided first-session setup | Import and confirm a profile; choose roles, locations, and remote preference; preview five matches; show the next action. Offer synthetic demo data. | A new user reaches a reviewed match through the dashboard without editing YAML. Import errors preserve the previous profile. Record time to first reviewed match locally if the user opts in. | Reliable errors (R12), existing import and matching APIs. |
| F02 | P1 | Explainable match cards | Show matching profile evidence, missing requirements, reason for filtering, listing freshness, and authorization uncertainty. Add useful/irrelevant feedback. | Every displayed reason links to profile or JD evidence. Unchecked authorization stays unknown. Feedback can be reviewed and exported. | Persisted match reasons; explicit authorization inputs; F01. |
| F03 | P1 | Unified application review queue | Show JD, resume diff, final letter, artifacts, and missing required fields together. Support approve, edit, skip, and retry. | Approval identifies the exact application/content version; editing invalidates it. Stale or repeated clicks do not send duplicates. Unsupported submitters offer a clear manual handoff. | R01–R07, R13. |
| F04 | P1 | Manual employer-outcome timeline | Record externally applied, replied, interview, rejection, offer, and withdrawal with timestamps and notes, separately from technical pipeline state. | Valid transitions are audited; external application entries are identified as manual; changing an outcome updates reminders and analytics consistently. | R08, R11, R15; submission-attempt identity. |
| F05 | P1 | Follow-up inbox | Show company/title, due date, full editable draft, snooze, dismiss, and mark handled. Provide copy/export for manual sending. | Repeated checks are idempotent; snoozes persist; replies stop inappropriate reminders; reviewing or handling a reminder does not automatically send email. | R09, R10, R18; F04. |
| F06 | P2 | Search that scales | Server-side search, source/state/remote filters, pagination, saved views, and visible counts. Retain existing browser-local saved searches. | Search works beyond the first 10,000 listings; saved filters restore accurately; stale requests cannot replace newer results; failed refreshes preserve previous results. | R17; existing workspace JavaScript. |
| F07 | P2 | Cost-controlled tailoring | Make local/hybrid/LLM selection explicit; show estimated batch work and actual usage; add configurable call/token budgets and profile-version-aware compiled content. | Local batches make zero LLM calls. Budget exhaustion stops additional paid work with a resumable report. Changed profiles invalidate compiled content. Estimates are labeled separately from observed usage. | R02, R05, R07; reconcile the LLM reduction plan. |
| F08 | P2 | Evidence-based interview workspace | Attach role-specific questions, profile-grounded STAR examples, notes, and feedback to an actual application. | Each suggested experience claim cites source profile material; editing a profile exposes stale preparation; feedback remains tied to its application. | F03/F04; existing interview and feedback modules. |
| F09 | P2 | Trustworthy search insights | Source/role response rates, time to response, rejected/withdrawn distinctions, and real content-variant comparisons. | All charts expose denominator, date window, sample size, and missing observations. Operational failures do not count as employer outcomes. | R11, R12, R16; F04. |
| F10 | P2 | Portable workspace export and restore | Export profile, listing history, approved content, artifacts, and outcomes; create a consistent database snapshot and validate restore in a separate workspace. | Round-trip restore preserves relationships and artifact references. Export scope is visible, secrets are excluded by default, and an existing workspace is not overwritten implicitly. | R07/R08; documented retention and schema-version handling. |

### Design constraints for the feature work

1. Preserve explicit approval for submissions and messages. A review, reminder, or analytics action must not send anything implicitly.
2. Keep resume and interview content grounded in the candidate's profile. Reused content needs both candidate-version and recipient checks.
3. Retain the current local-first workflow. Saved searches and measurements should work locally; external telemetry or mailbox integration requires a separately designed opt-in flow.
4. Keep eligibility uncertainty visible. The existing [eligibility helper](../crates/careerai-match/src/eligibility.rs) is not wired into the reviewed matching path, and substring checks such as “security clearance” do not distinguish mandatory, optional, and negated requirements. Improve its model before using it to reject jobs automatically.
5. Treat current legitimacy scores as heuristics. Expose reasons and uncertainty; do not present them as verified employer authenticity.
6. Keep hosted multi-user access, mailbox ingestion, autonomous outreach, and monetization experiments for a later plan with their own access, privacy, retention, and support requirements.

## Suggested delivery sequence

| Milestone | Work | Exit condition |
| --- | --- | --- |
| M1 — protect application correctness | R01–R08, R13–R15; add focused regression coverage. | Submission claims, approval state, recipient-correct reuse, local-mode guarantees, atomic stage changes, and recovery behavior are demonstrated with controlled fixtures. |
| M2 — make failures and reminders reliable | R09–R12, R18; complete the outcome model needed for follow-ups. | Database failures are visible, reminders are idempotent, and technical errors cannot masquerade as employer responses. |
| M3 — complete the daily workflow | F01–F05. | A user can import, review matches, approve an application, record an outcome, and handle a reminder through the dashboard. |
| M4 — improve scale and learning | R16/R17; F06–F10. | Large searches remain complete, budgets are enforced, insights show their evidence, and export/restore is verified. |

This is a dependency order, not a calendar commitment. Estimate each milestone after its schema changes, migration requirements, and test fixtures have been agreed.

## Engineering and verification backlog

- **Exercise failure paths:** database insert rejection, closed pools, missing payloads, corrupted artifacts, process timeout, concurrent submissions, and stale UI requests. Use temporary databases, synthetic profiles, and mock HTTP endpoints.
- **Protect real user data in tests:** isolate application roots and subprocess stubs; avoid process-global environment races. Live LLM/provider tests should be explicit and separate from deterministic tests.
- **Run relevant existing suites when implementation resumes:** database/pipeline/submit/scheduler/dashboard tests, the workspace JavaScript tests, and resume guardrail/render tests when changing content generation. A clean full-suite result is not asserted by this review document.
- **Add JavaScript checks to CI:** [workspace_ui.test.cjs](../crates/careerai-dashboard/tests/workspace_ui.test.cjs) exists, but [the CI workflow](../.github/workflows/ci.yml) does not invoke it. Include keyboard, saved-view, retry, stale-response, and service-worker boundaries as the UI grows.
- **Reduce dashboard coupling incrementally:** [index.tera](../crates/careerai-dashboard/templates/index.tera) contains 4,716 lines in this baseline. Extract coherent template partials and JavaScript modules when touching each workflow, retaining current behavior and regression coverage.
- **Reconcile older plans:** [PLAN_LLM_REDUCTION.md](../PLAN_LLM_REDUCTION.md) still marks foundations as unchecked although local tailoring, variants, and skeleton modules exist. Record implemented versus integrated versus validated status. Its example variants introduce new metrics/claims and should be replaced with fact-preserving examples. Re-measure call counts and costs; the current variant compiler makes calls per bullet, so “one call per profile” is not an established result.
- **Review migrations using populated fixtures:** cover follow-ups, variants, feedback, payloads, artifacts, and older application history; document how each new constraint treats existing data.

## Definition of done for each future task

An issue closes only when its behavior is corrected, its trigger has meaningful regression coverage, and related CLI/dashboard states agree. Feature completion requires a usable end-to-end path, visible empty/error states, keyboard access where applicable, and updated documentation. Record migration effects and remaining limitations alongside the validation result. Preserve pre-existing work and avoid labeling the software “bug-free” based on this focused review.
