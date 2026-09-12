# Hosted Beta Scope

Reference: [HOSTED_PRODUCT_DESIGN.md](HOSTED_PRODUCT_DESIGN.md) for the full
design document.

## Narrow vertical slice

The hosted beta serves invited individual candidates in one niche (AI/ML,
embedded, or robotics) and one region (India, remote-first/Delhi-NCR), with one
personal workspace per account.

## Beta inclusions

| # | Feature | Notes |
|---|---------|-------|
| 1 | Account and workspace creation | Profile import/confirmation, target role/company selection |
| 2 | Permitted ATS/feed discovery | Reviewed allowlist only; explainable matching and freshness |
| 3 | Target-company preparation program | Sourced company/role dossier, fit map, evidence-linked STAR stories, interview questions, recruiter questions, benefits/compensation checklist, due dates |
| 4 | Review-only tailored artifacts | Constrained-diff resume/cover-letter; preview and download, never live submission |
| 5 | Manual outcome tracking | Application, interview, email/call outcome records and follow-up reminders |
| 6 | Versioned encrypted export | User deletion request and deletion-status display |
| 7 | Entitlement admission | Free/paid usage visibility; no browser minutes or expensive work without entitlement reservation |

## Beta exclusions

- Live submission to any ATS or employer portal.
- All browser automation (LinkedIn, Indeed, Naukri, or otherwise).
- Mailbox OAuth/sync (Gmail, Outlook).
- Call recording or transcript storage.
- Document vault beyond generated artifacts (resume, cover letter).
- Tax cards, tax calculations, or tax-saving recommendations.
- Reimbursement processing or claim submission.
- Market-news aggregation.
- Coach/concierge access.
- Automatic communication (email send, message posting).

## Beta exit criteria

Measured over 30 days with at least 20 invited users:

| Metric | Target | Owner |
|--------|--------|-------|
| Onboarding completion | >= 60% | Product/Engineering lead |
| Median time to first reviewed match | < 10 minutes | Product/Engineering lead |
| Week-two return rate | >= 40% | Product/Engineering lead |
| Active users completing a preparation program | >= 30% | Product/Engineering lead |
| Generated claims with evidence or unresolved label | >= 80% | Product/Engineering lead |
| Cross-tenant test failures | 0 | Security |
| Live external writes | 0 | Security |
| API availability | >= 99.5% | Operations |
| p95 API latency (excluding queued work) | < 500 ms | Operations |
| RPO | <= 15 minutes | Operations |
| RTO | <= 4 hours | Operations |
| Restore drill (RPO/RTO demonstrated) | Pass | Operations |
| Gross contribution margin model | Completed with actual provider/support costs | Operations/Finance |

If any gate fails, remain invite-only or roll back the relevant feature flag.

## Release gates (documentation)

- Written security review (Security).
- Privacy notice and DPA (Legal/Privacy).
- Subprocessor inventory (Legal/Privacy).
- Source-permission register (Legal/Privacy).
- Incident runbook (Security/Operations).
- Deletion/export drill (Operations).

## Later milestone exit criteria

| Milestone | Gate | Owner |
|-----------|------|-------|
| M2 documents | Key hierarchy, malware quarantine, lifecycle, deletion verification tests; >= 99% upload scan completion; successful restore/delete drill | Security |
| M3 communications | Legal approval of exact OAuth scopes, token handling, retention, send policy; 100% outbound sends require reviewed approvals in tests | Legal/Privacy |
| M4 financial administration | User-entered records only; correction history, journal balancing, export reconciliation | Product/Privacy |
| M5 regional information | One named tax year/population; reviewed rule pack; stale/withdrawn rules fail closed; legal and named rules-owner sign-off | Legal/Privacy |
| M6 controlled side effects | Per-source written permission/terms decision, consent expiry, adapter contract tests, crash/reconciliation tests, live-action kill switch | Legal/Privacy/Security |

## Beta roles

| Role | Capabilities |
|------|-------------|
| `owner` | Full read/write to own workspace data; approve side effects (with reauth + consent); billing; export/delete; security audit timeline (own customer view) |
| `support_readonly` | Time-limited, reason-coded, resource-scoped, read-only access with explicit grant; no impersonation, no approval creation, no capability issuance |

No member/coach role is exposed until delegation is designed.
