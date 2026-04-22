CREATE TABLE channel_instances (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_kind TEXT NOT NULL CHECK (channel_kind = lower(channel_kind)),
    instance_key TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    is_primary BOOLEAN NOT NULL DEFAULT TRUE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_channel_instances_user_kind
    ON channel_instances (user_id, channel_kind);

CREATE INDEX idx_channel_instances_enabled
    ON channel_instances (enabled)
    WHERE enabled = TRUE;

CREATE UNIQUE INDEX idx_channel_instances_primary_per_kind
    ON channel_instances (user_id, channel_kind)
    WHERE is_primary = TRUE;
