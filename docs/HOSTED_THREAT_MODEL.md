# Hosted Threat Model

Reference: [HOSTED_PRODUCT_DESIGN.md](HOSTED_PRODUCT_DESIGN.md) for the full
design document.

## Trust boundaries

| Boundary | Inside | Outside | Enforcement |
|----------|--------|---------|-------------|
| API ingress | Authenticated Axum API with session cookies | Untrusted internet | TLS termination, rate limiting, CSRF protection |
| Tenant isolation | Per-tenant data rows | Other tenants' data | PostgreSQL RLS with `SET LOCAL app.tenant_id`; fail-closed on missing context |
| Worker boundary | Isolated Rust workers with signed queue envelopes | API process, external providers | Workers re-resolve tenant/entitlement in-transaction; no tenant ID from untrusted payload |
| Object storage | Tenant-scoped object-version rows | Raw object bytes | Capability-mediated access; RLS check before any byte read |
| Secrets vault | KMS/HSM-backed envelope encryption | Application logs, LLM prompts, exports | C3 data never leaves the vault; separate secrets key |
| Audit store | Append-only DB role | Application roles, migrator | Application roles cannot insert arbitrary event bodies; no update/delete on historical rows |
| Provider egress | Approved adapters with source-terms version | Unapproved endpoints, CAPTCHA, stealth | Source-terms register gates; kill switch per source |

## Attack surfaces

1. **Authentication**: Magic-link token theft, TOTP bypass, session fixation,
   cookie theft, recovery-code abuse.
2. **Authorization**: Cross-tenant data access (IDOR), role escalation,
   support-access abuse, workspace switching bypass.
3. **Input validation**: SQL injection (via unsanitized queries), path
   traversal (object keys), JSON canonicalization attacks, SSRF (provider
   URLs), XXE (pandoc input).
4. **Action protocol**: Replay of approved actions, blind retry after crash,
   idempotency-key collision, entitlement bypass, approval forgery.
5. **Object storage**: Cross-tenant object access, revoked capability replay,
   native-URL revocation gap, orphan object enumeration, forged capability.
6. **Audit integrity**: Direct insert of fabricated events, chain fork, missing
   event, replay, key compromise, restore poisoning.
7. **Provider abuse**: Terms-of-service violation, rate-limit evasion, cookie
   reuse after consent expiry, automated scraping detection.
8. **Supply chain**: Dependency vulnerability, malicious dependency, supply-chain
   compromise of build pipeline.

## Threat scenarios and mitigations

### T1: Cross-tenant data access (IDOR)

- **Scenario**: Attactor substitutes another tenant's ID in an API route,
  worker job, webhook, export, or support path.
- **Mitigation**: PostgreSQL RLS on every customer table; composite `(tenant_id,
  id)` foreign keys prevent cross-tenant references; `SET LOCAL app.tenant_id`
  set per-transaction from signed context; pool checkout resets connection;
  workers re-resolve membership in-transaction; uniform 404 for inaccessible
  reads (no enumeration).
- **Test**: Adversarial tests attempt another tenant's IDs through every route,
  worker, webhook, export, reporting, support, and object-storage path.

### T2: Session hijacking and cookie theft

- **Scenario**: Attacker steals a session cookie via XSS or network sniffing.
- **Mitigation**: Cookies are `Secure`, `HttpOnly`, `SameSite`; sessions are
  opaque hashes stored server-side; rotated after login and privilege change;
  idle timeout 30 minutes; absolute timeout 30 days; no bearer tokens in
  browser storage.
- **Test**: Session revocation, rotation, and timeout tests.

### T3: Action replay after crash or unknown provider result

- **Scenario**: Worker crashes mid-provider-call; system blindly retries,
  causing a duplicate side effect (e.g., double application submission).
- **Mitigation**: Every crash/timeout from `executing` becomes `unknown`; no
  retry from `unknown`; adapter-specific reconciliation must prove no side effect
  occurred before returning to an executable state; one-time, exact-payload
  approval with 10-minute expiry; fencing tokens on lease acquisition.
- **Test**: Crash tests kill the worker immediately before and after each
  network write and assert `unknown` with no blind retry.

### T4: Audit tampering

- **Scenario**: Operator or attacker edits, deletes, or fabricates audit events
  to hide unauthorized access.
- **Mitigation**: Append-only DB role for `security_audit_events`; application
  roles cannot insert arbitrary bodies; hash-chained events with
  `previous_hash` and `event_hash`; signed periodic anchors to WORM storage;
  independent `careerai-audit-verify` tool validates signatures, sequence
  continuity, and hash links without modifying data.
- **Test**: Fabricated direct inserts, missing events, concurrent writers,
  forked chains, duplicate/replay, mutation, deletion, rollback, key
  compromise, restore, and correction verification.

### T5: Object storage cross-tenant access

- **Scenario**: Attacker guesses or reuses an object key to access another
  tenant's file.
- **Mitigation**: Object keys are generated opaque UUIDs; every request loads
  the DB object-version row through RLS first; capabilities are tenant/object
  bound, signed, and short-lived; proxy for C1-C4; orphan scanner detects
  unexpected objects.
- **Test**: Cross-tenant access, revoked capabilities, replay/expiry, range
  mismatch, forged keys, object substitution, orphan detection.

### T6: Entitlement bypass

- **Scenario**: User triggers expensive work (LLM, browser, render) without a
  valid entitlement reservation.
- **Mitigation**: Admission transaction locks entitlement version, reserves
  estimated units, and enqueues only after commit; workers finalize actual
  units or release remainder; admission fails closed except for explicitly
  free/read/export operations.
- **Test**: Fail-closed admission, reserve/finalize/release, plan versioning.

### T7: Supply chain compromise

- **Scenario**: A dependency vulnerability or malicious crate is introduced.
- **Mitigation**: `cargo audit` CVE scan, `cargo deny` license and supply-chain
  policy, dependency pinning, `semgrep` baseline, periodic review.
- **Test**: CI runs `cargo audit`, `cargo deny check`, and `semgrep` on every
  PR.

### T8: Provider terms violation

- **Scenario**: System scrapes or automates a source without written permission,
  causing legal or reputational harm.
- **Mitigation**: Source-permission register gates every adapter; per-source
  kill switch; consent expiry; browser automation disabled in beta; prohibited
  behaviors explicitly listed (CAPTCHA bypass, stealth, unsupported endpoints).
- **Test**: Adapter contract tests verify permitted use; kill switch tests.

## Security launch gates

1. Threat modeling session completed and documented.
2. Dependency scan (`cargo audit`), SAST, and secret scan clean.
3. RLS and adversarial tenant-isolation tests green.
4. Authentication and session management tests green.
5. Action protocol crash tests green.
6. Malware quarantine tests green.
7. Export, deletion, and restore drill completed.
8. Access review (support grants, break-glass) completed.
9. Incident response exercise completed.
10. Legal and privacy approval (regional, vendor, DPA, subprocessor inventory).

## Preserved local safety invariants

The hosted product preserves all existing local safety invariants:

- **Constrained resume diffs**: `careerai-tailor/src/diff.rs` schema validation
  rejects any diff op outside the grammar. No invented experience, titles,
  dates, or employers.
- **Dry-run defaults**: `auto_submit` defaults to `false`; dry-run submitters
  never issue network writes.
- **Per-source gates**: `submit_enabled` honored even with `--auto-submit`.
- **Rate limiting**: All outbound submissions go through `governor` token
  buckets; quiet hours and daily caps enforced at the boundary.
- **Credential redaction**: `tracing` redaction filter; regex-based test asserts
  no known-secret shapes in captured events.
- **Prompt injection mitigations**: LLM inputs are sandboxed; constrained
  grammar limits LLM output to reordering/rewriting existing bullets.
