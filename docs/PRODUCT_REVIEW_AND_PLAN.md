# CareerAI product review and delivery plan

Review date: 2026-09-08. Scope: the current working tree, including existing uncommitted features. This is a focused architecture and code review of the dashboard, discovery/matching surfaces, pipeline execution, database transitions, and safety boundaries; it is not an exhaustive security audit of every adapter.

## Product direction

Make CareerAI a daily job-search workspace for technical candidates: find a small number of credible matches, explain the fit, prepare an honest application, and track what happens next. The differentiator to validate is a private, evidence-based workflow with user control over submission. More application volume alone is a weak success metric.

The current product already has discovery adapters, matching, constrained resume tailoring, rendering, submission gates, referrals, follow-up scheduling, interview feedback, and analytics. Prioritize completing these journeys over adding more unrelated modules. The dashboard currently spends too much space explaining the product before showing the user's work.

## This delivery

- Replace the promotional overview with a responsive workspace: compact navigation, an actionable daily focus, real pipeline counts, a weekly application target, and a seven-day activity view.
- Add a keyboard command palette and named saved searches, with browser-local persistence and explicit empty/error states.
- Use native dialogs, CSS motion, and progressive enhancement without a frontend framework migration or runtime CDN dependencies.
- Fix explorer loading/filtering/refresh defects, unchecked eligibility labels, unsafe posting links, and service-worker cache boundaries.
- Preserve current submission gates and all pre-existing working-tree changes.

Implementation and verification results are recorded below when complete.

## Roadmap and validation

| Priority / phase | Feature | Why it matters | Acceptance / experiment |
| --- | --- | --- | --- |
| P0 · weeks 1–2 | Trust and reliability | Incorrect state, hidden errors, and invented status erode confidence | Explicit failures and retries; coherent stage transitions; dry-run never counts as a sent application; regression coverage |
| P0 · weeks 1–2 | First-session onboarding | A user should reach a useful shortlist without learning the CLI | Import → confirm profile → choose target role/location → preview five matches; measure time to first reviewed match |
| P1 · weeks 3–4 | Explainable match cards | A percentage alone is difficult to act on | Show supporting profile evidence, missing requirements, eligibility uncertainty, and source freshness; users mark useful / irrelevant |
| P1 · weeks 3–4 | Review queue | Helps finish applications rather than accumulate drafts | Resume diff, job description, cover letter and missing fields together; explicit approval; retry without duplicate submissions |
| P1 · weeks 5–6 | Outcome tracking | Turns the pipeline into a learning loop | Manual applied / replied / interview / offer tracking first; timestamps and notes; optional email import only with consent and a privacy design |
| P1 · weeks 5–6 | Follow-up inbox | Prevents good applications from getting forgotten | Company/title context, due date, edit draft, snooze, mark handled; never send messages automatically from a review action |
| P2 · weeks 7–8 | Evidence-based interview prep | Reuses the candidate's real experience | Role-specific questions and STAR stories grounded in profile bullets; feedback tied to an actual application |
| P2 · weeks 7–8 | Search quality insights | Helps users change strategy | Response rates by source and role with sample sizes; no confident A/B conclusions on tiny samples |
| Later | Hosted access and integrations | Potentially expands the audience | First design authentication, tenant isolation, retention/deletion, backup/restore, and operational ownership |

## A path to adoption

1. Recruit 10–15 candidates in one niche (e.g. embedded/robotics engineers or AI application engineers). Observe onboarding and one week of real usage before widening the audience.
2. Run a four-week pilot. Interview users who stop returning. Test whether they find better matches and finish applications with less editing.
3. Publish a short, reproducible demo with synthetic candidate data and a sample workspace. Explain what stays local, what reaches an LLM, and what requires approval.
4. Test a paid convenience tier only after repeat usage: managed setup, reliable scheduled discovery, and polished exports. Treat willingness to pay as an experiment, not an assumption.
5. Share through niche engineering communities and career coaches with permission. Keep bounties and generic productivity features secondary until core retention is established.

Suggested measurements (targets to validate, not current results): median time to first reviewed match under 10 minutes; 60% of pilot users complete onboarding; 40% return in week two. North-star candidate: qualified employer conversations per weekly active candidate. Also track user-rated shortlist relevance, edit time per application, response rate with sample size, failures, duplicate submissions, and LLM cost per reviewed application. Collect telemetry only with opt-in; support local measurement/export first.

## Design references

- [W3C native modal dialogs](https://www.w3.org/WAI/WCAG21/Techniques/html/H102): keyboard containment, Escape, and focus restoration for the command palette.
- [MDN reduced motion](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/@media/prefers-reduced-motion): reduce nonessential animation when requested.
- [MDN View Transition API](https://developer.mozilla.org/en-US/docs/Web/API/View_Transition_API): optional enhancement for navigation, with immediate switching as the fallback.

## Review findings and verification

To be completed with code references, dispositions, and checks from this delivery.
