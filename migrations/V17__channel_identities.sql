-- Maps a channel-specific external identity to an IronClaw owner.
-- This is how inbound messages are resolved to the right user's resources.
CREATE TABLE channel_identities (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel     TEXT        NOT NULL,
    external_id TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (channel, external_id)
);

CREATE INDEX idx_channel_identities_lookup
    ON channel_identities (channel, external_id);
