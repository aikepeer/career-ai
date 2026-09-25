# Hosted Unverified Dependency Register

Reference: [HOSTED_PRODUCT_DESIGN.md](HOSTED_PRODUCT_DESIGN.md) for the full
design document.

Each item below is an external dependency, integration, or vendor claim that
the design document marks as **unverified pending vendor/legal review**. No
item may be used in a paid SLA or feature flag until its gate is satisfied and
the owner signs off.

## Register

| # | Dependency / claim | Gate | Owner | Status |
|---|--------------------|------|------|--------|
| 1 | Merchant-of-record (Stripe or alternative) | Written contract confirming MoR responsibility for GST/VAT, invoices, refunds, disputes, and support | Operations / Legal | Pending |
| 2 | Stripe Billing behavior | Vendor confirmation of tax/refund/support responsibilities, retention policy, webhook event ordering | Operations / Legal | Pending |
| 3 | ATS/feed sources (beta allowlist) | Written paid-use permission for each source in the beta allowlist | Legal / Privacy | Pending |
| 4 | LinkedIn browser automation | Written terms/permission decision; not approved for hosted beta | Legal / Privacy | Pending |
| 5 | Indeed browser automation | Written terms/permission decision; not approved for hosted beta | Legal / Privacy | Pending |
| 6 | Naukri browser automation | Written terms/permission decision; not approved for hosted beta | Legal / Privacy | Pending |
| 7 | Gmail / Outlook mailbox OAuth | Legal approval of exact OAuth scopes, token handling, retention, send policy | Legal / Privacy | Pending |
| 8 | Calendar integration | Vendor terms, scopes, data retention/deletion limits | Legal / Privacy | Pending |
| 9 | LLM provider retention / DPA | Confirmation that each model provider's retention/DPA terms permit each data class (C0-C4) | Legal / Privacy | Pending |
| 10 | Cloud KMS / object-store guarantees | Vendor SLA, key management practices, deletion guarantees, regional residency | Security / Operations | Pending |
| 11 | India tax year / population / rules owner | Named rules owner and legal sign-off for M5 regional information pack | Legal / Privacy | Pending |
| 12 | Jurisdictional tax / privacy conclusions | Regional regulatory review for data residency and customer-notification commitments | Legal / Privacy | Pending |
| 13 | Data residency commitments | Operations funding and vendor confirmation of regional residency | Operations / Legal | Pending |
| 14 | Support subprocessor (if non-employee) | Approved subprocessor with confidentiality terms, DPA, access controls | Legal / Privacy / Security | Pending |
| 15 | Source-specific live action approval | Per-source written terms decision, consent expiry, adapter contract tests, crash/reconciliation tests | Legal / Privacy / Security | Pending |
| 16 | Browser automation (hosted) | Separate approval beyond beta; not enabled by default | Security / Legal | Pending |
| 17 | Community scrapers / undocumented endpoints | Reviewed and permitted or rejected; no undocumented API use | Engineering / Legal | Pending |

## Gate requirements per item

Each gate requires the following before the dependency is approved:

1. **Signed terms, DPA, or documented provider guarantee** covering: scopes,
   data retention/deletion limits, rate limits, incident contact, cost model,
   and source-terms version.
2. **Legal/Privacy review** confirming regional and regulatory applicability.
3. **Security review** confirming threat-model coverage and mitigations.
4. **Owner sign-off** recorded in this register with date and decision.

## Beta-critical dependencies

Only items 1, 2, 3, 9, and 10 are beta-critical (required before paid beta).
Items 4-8 and 11-17 are post-beta and must not block beta launch. Their feature
flags default off and their schemas/code paths are not loaded by beta jobs.

## Open questions

These open questions from the design document map to register items:

1. Which initial merchant-of-record/provider and contractually verified
   tax/refund/support responsibilities will be used? (Item 1, 2)
2. Which ATS/feed sources have written paid-use permission for beta? (Item 3)
3. Which India population, tax year, and rules owner — if any — will approve
   M5? (Item 11)
4. Which model providers' retention/DPA terms permit each data class? (Item 9)
5. Is support access employee-only, or will an approved subprocessor be needed?
   (Item 14)
6. What data residency and customer-notification commitments can Operations
   fund? (Items 12, 13)
7. When, if ever, will a source-specific live action or browser automation be
   legally approved? (Items 4-6, 15, 16)
