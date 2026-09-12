-- 003_external_actions.sql
-- External action state machine: tracks every external side effect.
-- States: created → approved → leased → executing → succeeded | unknown
--         unknown → no_side_effect_confirmed → approved (fresh)
--         unknown → side_effect_confirmed | permanently_failed
--         approved → expired | rejected

CREATE TABLE external_actions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    actor_id UUID NOT NULL,
    action_type TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    payload_schema_version INTEGER NOT NULL DEFAULT 1,
    canonical_payload JSONB NOT NULL,
    state TEXT NOT NULL DEFAULT 'created',
    approval_actor UUID,
    approval_time TIMESTAMPTZ,
    approval_expiry TIMESTAMPTZ,
    lease_token TEXT,
    fencing_token BIGINT,
    attempt_id BIGINT,
    idempotency_key TEXT,
    consent_id TEXT,
    consent_version INTEGER,
    source_terms_version TEXT,
    entitlement_reservation_id UUID,
    policy_snapshot JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- State history for audit trail
CREATE TABLE external_action_transitions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    action_id UUID NOT NULL REFERENCES external_actions(id),
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    actor TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes
CREATE INDEX idx_external_actions_tenant ON external_actions(tenant_id);
CREATE INDEX idx_external_actions_state ON external_actions(state);
CREATE INDEX idx_external_actions_idempotency ON external_actions(idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- RLS
ALTER TABLE external_actions ENABLE ROW LEVEL SECURITY;
ALTER TABLE external_actions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_ext_actions ON external_actions
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE external_action_transitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE external_action_transitions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_ext_transitions ON external_action_transitions
    USING (action_id IN (
        SELECT id FROM external_actions
        WHERE tenant_id::text = current_setting('app.tenant_id', true)
    ));

-- Check constraint: valid state transitions enforced at application level
-- (state machine logic in Rust action::state module)
