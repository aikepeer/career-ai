# Incident Runbook — careerai-hosted Beta

## Severity levels

| Level | Definition | Response time | Owner |
|---|---|---|---|
| SEV1 | Data loss, cross-tenant access, security breach | Immediate | Security + Engineering |
| SEV2 | Service down, billing failure, queue stuck | <15 min | Engineering on-call |
| SEV3 | Degraded performance, single tenant issue | <1 hour | Engineering on-call |
| SEV4 | Minor bug, cosmetic issue | Next business day | Engineering |

## Kill switches

### Pause all workers
```
POST /v1/admin/workers/pause
```
Effect: stops processing queued jobs. Already-running jobs complete. No new jobs are leased.

### Force dry-run for all submissions
```
POST /v1/admin/submissions/dry-run
```
Effect: all submission actions return `would_submit` without network writes, regardless of user setting.

### Revoke all sessions (force re-login)
```
POST /v1/admin/sessions/revoke-all
```
Effect: all active sessions are revoked. Users must re-authenticate.

### Revoke webhook secrets
```
POST /v1/admin/webhooks/revoke
```
Effect: clears all billing webhook secrets. Incoming webhooks are rejected until reconfigured.

## Incident response procedure

1. **Page owner** — On-call engineer acknowledges within response time.
2. **Contain** — Apply kill switches as needed (pause queue, force dry-run, revoke keys).
3. **Preserve evidence** — Snapshot logs, database state, queue contents before remediation.
4. **Assess scope** — Which tenants are affected? Is data at risk?
5. **Notify** — Security/Legal if SEV1 or SEV2. Customers within contractual/legal window.
6. **Remediate** — Fix the root cause. Roll back if necessary.
7. **Postmortem** — Within 48 hours. What happened, timeline, root cause, action items.

## Backup and restore

### Backup schedule
- **Database**: daily full + continuous WAL archiving
- **Object storage**: daily incremental
- **Audit anchors**: every 5 minutes to WORM storage
- **Retention**: 35 days

### Restore drill (monthly)
1. Provision a fresh PostgreSQL instance from the latest backup.
2. Verify RLS policies are intact (`SELECT * FROM pg_policies WHERE schemaname = 'public'`).
3. Run `careerai-audit-verify` against the restored database.
4. Confirm all tenant data is present and isolated.
5. Record RPO (data freshness) and RTO (restore time).
6. Beta targets: RPO ≤ 15 minutes, RTO ≤ 4 hours.

## Deletion drill

1. Create a test tenant with profile, listings, artifacts, outcomes.
2. Request account deletion via `DELETE /v1/account`.
3. Verify:
   - Session is revoked immediately.
   - Primary rows are marked for deletion.
   - Object versions are marked deleted.
   - Signed URLs/capabilities are invalidated.
   - Worker scratch is wiped.
   - Audit records retain only pseudonymous deletion receipt.
4. Wait for retention period (or trigger manual purge).
5. Verify all data is physically removed.

## Alert routing

| Alert | Pager | Channel |
|---|---|---|
| Cross-tenant test failure | SEV1 | Security on-call |
| Missing tenant context | SEV1 | Security on-call |
| Queue backlog > 1000 | SEV2 | Engineering on-call |
| Backup failure | SEV2 | Engineering on-call |
| RPO breach > 15 min | SEV2 | Engineering on-call |
| Webhook drift | SEV3 | Engineering on-call |
| Cost spike > 2x daily avg | SEV3 | Engineering on-call |

## Contact roster

| Role | Name | Contact |
|---|---|---|
| Engineering on-call | (configured in pager rotation) | PagerDuty |
| Security owner | TBD | TBD |
| Legal/Privacy | TBD | TBD |
| Operations | TBD | TBD |

## Beta exit criteria checklist

- [ ] 60% onboarding completion rate (≥20 invited users over 30 days)
- [ ] Median time to first reviewed match < 10 minutes
- [ ] 40% week-two return rate
- [ ] 30% of active users complete a preparation program
- [ ] ≥80% of generated claims have evidence or explicit unresolved label
- [ ] Zero cross-tenant test failures
- [ ] Zero live external writes
- [ ] 99.5% API availability
- [ ] p95 API latency < 500 ms (excluding queued work)
- [ ] RPO ≤ 15 minutes, RTO ≤ 4 hours (demonstrated by restore drill)
- [ ] Gross contribution margin model completed with actual provider/support costs
- [ ] Written security review
- [ ] Privacy notice / DPA
- [ ] Subprocessor inventory
- [ ] Source-permission register
- [ ] Incident runbook (this document)
- [ ] Deletion/export drill completed

If any gate fails, remain invite-only or rollback the relevant feature flag.
