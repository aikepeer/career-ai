-- 004_entitlements_billing.sql
-- Entitlements, usage reservations, and billing events.

-- Entitlements: per-tenant plan limits
CREATE TABLE entitlements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    plan_id TEXT NOT NULL,
    plan_version INTEGER NOT NULL DEFAULT 1,
    feature TEXT NOT NULL,
    limit_units BIGINT NOT NULL,
    used_units BIGINT NOT NULL DEFAULT 0,
    reserved_units BIGINT NOT NULL DEFAULT 0,
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(tenant_id, feature, period_start)
);

-- Usage reservations: pending unit reservations
CREATE TABLE usage_reservations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    entitlement_id UUID NOT NULL REFERENCES entitlements(id),
    plan_id TEXT NOT NULL,
    plan_version INTEGER NOT NULL,
    feature TEXT NOT NULL,
    reserved_units BIGINT NOT NULL,
    reserved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    finalized BOOLEAN NOT NULL DEFAULT false,
    released BOOLEAN NOT NULL DEFAULT false
);

-- Billing events: webhook receipts
CREATE TABLE billing_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID,
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_version INTEGER NOT NULL DEFAULT 1,
    customer_id TEXT,
    subscription_id TEXT,
    amount_cents BIGINT,
    currency TEXT,
    occurred_at TIMESTAMPTZ,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    status TEXT NOT NULL DEFAULT 'pending',
    raw_payload TEXT NOT NULL,
    signature_verified BOOLEAN NOT NULL DEFAULT false,
    UNIQUE(provider, event_id)
);

-- Price versions: provider-neutral pricing
CREATE TABLE price_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id TEXT NOT NULL,
    price_cents BIGINT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'usd',
    tax_inclusive BOOLEAN NOT NULL DEFAULT false,
    effective_from TIMESTAMPTZ NOT NULL,
    effective_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes
CREATE INDEX idx_entitlements_tenant ON entitlements(tenant_id);
CREATE INDEX idx_reservations_tenant ON usage_reservations(tenant_id);
CREATE INDEX idx_reservations_expiry ON usage_reservations(expires_at)
    WHERE finalized = false AND released = false;
CREATE INDEX idx_billing_events_customer ON billing_events(customer_id);

-- RLS
ALTER TABLE entitlements ENABLE ROW LEVEL SECURITY;
ALTER TABLE entitlements FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_entitlements ON entitlements
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE usage_reservations ENABLE ROW LEVEL SECURITY;
ALTER TABLE usage_reservations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_reservations ON usage_reservations
    USING (tenant_id::text = current_setting('app.tenant_id', true));

-- Billing events are webhook-managed; RLS by tenant_id when present
ALTER TABLE billing_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing_events FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_billing ON billing_events
    USING (tenant_id IS NULL OR tenant_id::text = current_setting('app.tenant_id', true));
