-- Maps a channel-specific external identity to an IronClaw owner.
-- This is how inbound messages are resolved to the right user's resources.
CREATE TABLE channel_identities (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel     TEXT        NOT NULL CHECK (channel = LOWER(channel)),
    external_id TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (channel, external_id)
);

-- UNIQUE (channel, external_id) already creates an implicit index on (channel, external_id).
