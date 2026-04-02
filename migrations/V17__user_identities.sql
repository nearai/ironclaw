-- Linked external identities for OAuth/social login.
--
-- Maps provider-specific user IDs (Google `sub`, GitHub user ID, etc.)
-- to internal user records, enabling multi-provider login and automatic
-- account linking by verified email.

CREATE TABLE user_identities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,                 -- 'google', 'github', 'apple', 'near', 'email'
    provider_user_id TEXT NOT NULL,         -- provider-specific unique ID
    email TEXT,                             -- email from the provider (for linking)
    email_verified BOOLEAN NOT NULL DEFAULT false,
    display_name TEXT,                      -- provider-sourced display name
    avatar_url TEXT,                        -- provider-sourced avatar URL
    raw_profile JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider, provider_user_id)
);

CREATE INDEX idx_user_identities_user ON user_identities(user_id);
CREATE INDEX idx_user_identities_email ON user_identities(email) WHERE email IS NOT NULL;
