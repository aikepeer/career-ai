-- 001_hosted_init.sql
-- Hosted PostgreSQL schema: tenants, users, sessions, workspaces, magic links, recovery codes.
-- All customer tables have tenant_id. RLS enabled in migration 002.

-- Extensions
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Tenants: one per workspace
CREATE TABLE tenants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Users: email-based login with magic link + TOTP
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT false,
    totp_secret TEXT, -- base32-encoded, NULL until enrolled
    mfa_enrolled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant_id UUID REFERENCES tenants(id)
);

-- Sessions: opaque hashed tokens
CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash TEXT NOT NULL UNIQUE,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    workspace_id UUID NOT NULL,
    role TEXT NOT NULL DEFAULT 'owner',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_active_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked BOOLEAN NOT NULL DEFAULT false
);

-- Workspaces
CREATE TABLE workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by UUID NOT NULL REFERENCES users(id)
);

-- Workspace memberships
CREATE TABLE workspace_memberships (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id),
    user_id UUID NOT NULL REFERENCES users(id),
    role TEXT NOT NULL DEFAULT 'owner',
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(workspace_id, user_id)
);

-- Magic link tokens
CREATE TABLE magic_link_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash TEXT NOT NULL UNIQUE,
    login_tx_id TEXT NOT NULL,
    email TEXT NOT NULL,
    user_agent_hash TEXT NOT NULL,
    client_ip TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT false,
    requested_workspace UUID
);

-- Recovery codes
CREATE TABLE recovery_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    set_id TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    used BOOLEAN NOT NULL DEFAULT false,
    invalidated BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Profiles: imported user profile data
CREATE TABLE profiles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    profile_data JSONB NOT NULL DEFAULT '{}',
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Listings: job listings from permitted sources
CREATE TABLE listings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    url TEXT,
    company TEXT,
    location TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_created_at TIMESTAMPTZ,
    UNIQUE(tenant_id, source, external_id)
);

-- Applications: user's application records
CREATE TABLE applications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    listing_id UUID REFERENCES listings(id),
    status TEXT NOT NULL DEFAULT 'discovered',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Application events: state transition audit trail
CREATE TABLE application_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    application_id UUID NOT NULL REFERENCES applications(id),
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Outcomes: manual outcome records
CREATE TABLE outcomes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    application_id UUID NOT NULL REFERENCES applications(id),
    outcome_type TEXT NOT NULL, -- interview, email, call, offer, rejection
    notes TEXT,
    occurred_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Follow-up reminders
CREATE TABLE follow_ups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    application_id UUID NOT NULL REFERENCES applications(id),
    due_at TIMESTAMPTZ NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT false,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
