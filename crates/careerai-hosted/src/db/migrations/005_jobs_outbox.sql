-- 005_jobs_outbox.sql
-- Durable job queue with outbox pattern and worker leases.
-- At-least-once delivery with fencing tokens, exponential backoff,
-- and dead-letter after poison-job classification.

-- Outbox: messages to be delivered to workers
CREATE TABLE outbox (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ
);

-- Job queue: durable jobs for workers
CREATE TABLE jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    job_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    signed_context TEXT NOT NULL, -- signed tenant context for worker
    entitlement_reservation_id UUID,
    state TEXT NOT NULL DEFAULT 'pending',
    -- pending → leased → running → succeeded | failed | dead_letter
    attempt INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    leased_at TIMESTAMPTZ,
    lease_expires_at TIMESTAMPTZ,
    fencing_token BIGINT,
    leased_by TEXT,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

-- Indexes for job processing
CREATE INDEX idx_jobs_pending ON jobs(created_at)
    WHERE state = 'pending';
CREATE INDEX idx_jobs_leased_expiry ON jobs(lease_expires_at)
    WHERE state = 'leased';
CREATE INDEX idx_jobs_tenant ON jobs(tenant_id);

-- Dead letter queue
CREATE TABLE dead_letter_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    original_job_id UUID NOT NULL,
    job_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    failure_reason TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    original_created_at TIMESTAMPTZ NOT NULL,
    dead_lettered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- RLS
ALTER TABLE outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE outbox FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_outbox ON outbox
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE jobs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_jobs ON jobs
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE dead_letter_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE dead_letter_jobs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_dlq ON dead_letter_jobs
    USING (tenant_id::text = current_setting('app.tenant_id', true));
