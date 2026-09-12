# Hosted Data Classification

Reference: [HOSTED_PRODUCT_DESIGN.md](HOSTED_PRODUCT_DESIGN.md) for the full
design document.

## Classification levels

| Class | Name | Examples | Default processing | Retention / deletion |
|-------|------|----------|--------------------|----------------------|
| C0 | Public | Listing text, public news, public company pages | Provider worker allowed | Source/license policy |
| C1 | Personal | Profile, resume, cover letter, interview answers, preparation tasks | Encrypted hosted processing with consent | Active account + user policy |
| C2 | Sensitive | ID/onboarding documents, offer letters, receipts, compensation records | Server-readable only when feature enabled; no default LLM | User-controlled; strict access |
| C3 | Secret | OAuth refresh tokens, browser cookies, provider API keys | Secrets vault only; never logs/LLM/export | Revoke then cryptographic destruction |
| C4 | Restricted | Recordings, support exports, legal-hold data | Disabled by default; explicit regional consent required | Shortest justified period or legal hold |

## Processing rules

- **C0**: May be processed by provider workers (discovery, matching) without
  additional consent. Source/license retention policy governs deletion.
- **C1**: Encrypted at rest with per-tenant DEKs. Hosted LLM processing is
  opt-in by data class and provider retention policy. User consent required.
- **C2**: Server-readable only when the owning feature module is explicitly
  enabled. No LLM processing by default. Strict access controls apply.
- **C3**: Stored exclusively in the secrets vault (KMS/HSM-backed envelope
  encryption with a separate secrets key). Never appear in logs, LLM prompts,
  exports, or audit event bodies. Revocation triggers cryptographic destruction.
- **C4**: Disabled by default. Requires explicit regional consent confirmed for
  all participants (e.g., recordings). Shortest justified retention period.

## Encryption architecture

- **KMS/HSM root key** (regional) wraps per-tenant data-encryption keys (DEKs).
- **Per-object DEKs** encrypt individual files with AEAD and authenticated
  metadata.
- **Separate secrets key** encrypts OAuth tokens and credentials (C3).
- Key rotation: wrapping keys rotated annually and immediately on incident.
  Rewrapping DEKs does not require rewriting object bytes.
- Tenant key material is destroyed after retention/legal-hold release, subject
  to documented backup expiry.

## Object storage authorization

Object bytes are never authorized by an object key alone. Every request:

1. Loads the tenant-scoped database object-version row through RLS.
2. Checks purpose, user/action, classification, lifecycle state, and revocation.
3. Issues a capability bound to `{tenant_id, object_version_id, user_id,
   action_id or purpose, HTTP method, permitted byte range, issued_at,
   expires_at, capability_version}`, signed with a server-held capability key.

- **Proxy** all C1-C4 content (and C0/C1 when immediate revocation is required).
  Proxy capabilities expire in 5 minutes and are checked on every request,
  providing immediate revocation.
- **Storage-native signed URLs** only for low-risk C0/C1 downloads after the
  same DB check. Maximum TTL 5 minutes. Revocation is bounded by that TTL; an
  already-issued URL cannot be invalidated before expiry. Prohibited for
  deleted, legal-hold, or incident-response objects.
- Object keys use generated opaque prefixes (`tenant UUID / object UUID /
  version UUID`), never user filenames or path traversal characters.
- A daily orphan scanner compares object inventory to live DB versions and
  quarantines unexpected objects.

## Upload safety

Uploads enter quarantine, verify magic bytes and declared type, scan for
malware, optionally disarm active content, and become available only after a
clean result. Rendering and LLM workers receive least-privilege, time-limited
access and wipe scratch space after use.

## Export

Exports are encrypted to a user-provided passphrase or public key and include a
manifest with schema version, record counts, SHA-256 checksums, source
provenance, and Merkle root. Export is reauthentication-protected.

## Deletion

Deletion marks a workflow that:

1. Revokes sessions and tokens.
2. Removes primary rows and objects.
3. Invalidates caches and signed URLs.
4. Deletes worker scratch space.
5. Records completion.

Deletion is complete when primary/replicas, object versions, search indexes,
caches, and eligible backup generations have passed their purge window.
Backups expire within 35 days. Immutable security audit records retain only
minimum pseudonymous metadata and a deletion receipt, not content. Legal holds
suspend deletion and are visible to the user.

## Support access matrix

| Class | Metadata | Content read | Download | Mutation/approval |
|-------|----------|-------------|----------|-------------------|
| C0 | Scoped yes | Scoped yes | No by default | No |
| C1 | Scoped yes | No by default; consent + redacted view only | Never | No |
| C2 | Minimal metadata only | Denied | Denied | Never |
| C3 | Existence/health only | Denied | Denied | Never |
| C4 | Existence/retention only | Denied | Denied | Never |

Support grants are resource-scoped, read-only, max 30 minutes, automatically
revoked, session-recorded where technically possible, fully audit-chained, and
notify the customer at grant and closure. Monthly review checks ticket/reason,
scope, timestamps, and notification; violations revoke the role.
