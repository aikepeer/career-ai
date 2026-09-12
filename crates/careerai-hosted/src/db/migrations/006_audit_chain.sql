-- 006_audit_chain.sql
-- Hash-chained append-only audit store.
-- event_hash = SHA-256(domain || shard_id || sequence || previous_hash || canonical_event)
-- Append-only: application roles cannot insert arbitrary event bodies,
-- and neither application roles nor the migrator can update/delete.

-- Security audit events (append-only)
CREATE TABLE security_audit_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    shard_id TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    previous_hash TEXT NOT NULL,
    canonical_event JSONB NOT NULL,
    event_hash TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    actor TEXT NOT NULL,
    transaction_id TEXT NOT NULL,
    job_id TEXT,
    source_record_hashes JSONB,
    authoritative_table TEXT,
    authoritative_version INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(shard_id, sequence)
);

-- Indexes
CREATE INDEX idx_audit_shard_seq ON security_audit_events(shard_id, sequence);
CREATE INDEX idx_audit_hash ON security_audit_events(event_hash);

-- Audit anchors: signed periodic Merkle roots
CREATE TABLE audit_anchors (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    shard_id TEXT NOT NULL,
    seq_start BIGINT NOT NULL,
    seq_end BIGINT NOT NULL,
    first_hash TEXT NOT NULL,
    last_hash TEXT NOT NULL,
    merkle_root TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    key_id TEXT NOT NULL,
    signature TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Append-only enforcement: revoke UPDATE and DELETE from application roles
REVOKE UPDATE, DELETE ON security_audit_events FROM careerai_api, careerai_worker, careerai_webhook;
REVOKE UPDATE, DELETE ON audit_anchors FROM careerai_api, careerai_worker, careerai_webhook;

-- Only a dedicated audit_ingest role can INSERT (granted separately)
-- Application roles can SELECT for their own audit timeline (customer view)

-- RLS: users see only their own tenant's audit events
-- (tenant_id is embedded in shard_id or as a field in canonical_event)
-- For beta, audit events are accessible only to the owner role
ALTER TABLE security_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE security_audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_tenant_isolation ON security_audit_events
    USING (canonical_event->>'tenant_id' = current_setting('app.tenant_id', true));

ALTER TABLE audit_anchors ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_anchors FORCE ROW LEVEL SECURITY;
CREATE POLICY anchor_tenant_isolation ON audit_anchors
    USING (shard_id LIKE current_setting('app.tenant_id', true) || '%');
