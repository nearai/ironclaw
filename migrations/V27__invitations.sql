-- Pilot onboarding invitations. Plaintext invite tokens are returned once
-- by the create API and are never stored.

CREATE TABLE invitations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token_prefix TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_by_admin TEXT NOT NULL,
    target_email TEXT,
    target_role TEXT NOT NULL DEFAULT 'user',
    scopes JSONB NOT NULL DEFAULT '{}',
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ,
    claimed_by_user_id TEXT,
    revoked_at TIMESTAMPTZ,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_invitations_token_hash ON invitations(token_hash);
CREATE INDEX idx_invitations_admin ON invitations(created_by_admin);
CREATE INDEX idx_invitations_expires ON invitations(expires_at)
    WHERE claimed_at IS NULL AND revoked_at IS NULL;
