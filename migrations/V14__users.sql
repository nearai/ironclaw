-- User management tables for multi-tenant deployments.
--
-- Replaces the static GATEWAY_USER_TOKENS env var with DB-backed
-- user registration, API token management, and invitation flow.

CREATE TABLE users (
    id TEXT PRIMARY KEY,                        -- matches existing user_id pattern (string, not UUID)
    email TEXT UNIQUE,                          -- nullable for token-only users
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',       -- active | suspended | deactivated
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login_at TIMESTAMPTZ,
    created_by TEXT REFERENCES users(id),       -- who invited this user (nullable for bootstrap)
    metadata JSONB NOT NULL DEFAULT '{}'        -- extensible profile data
);

CREATE TABLE api_tokens (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BYTEA NOT NULL,                  -- SHA-256 hash (never store plaintext)
    token_prefix TEXT NOT NULL,                 -- first 8 hex chars for display
    name TEXT NOT NULL,                         -- human label ("my-laptop", "ci-bot")
    expires_at TIMESTAMPTZ,                     -- nullable = never expires
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ                      -- soft-revoke: set this instead of deleting
);
CREATE INDEX idx_api_tokens_user ON api_tokens(user_id);
CREATE INDEX idx_api_tokens_hash ON api_tokens(token_hash);

CREATE TABLE invitations (
    id UUID PRIMARY KEY,
    email TEXT,                                  -- nullable for link-based invites
    invite_token_hash BYTEA NOT NULL,           -- SHA-256 hash of the invite link token
    invited_by TEXT NOT NULL REFERENCES users(id),
    status TEXT NOT NULL DEFAULT 'pending',      -- pending | accepted | expired | revoked
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    accepted_by TEXT REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
