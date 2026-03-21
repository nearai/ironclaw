-- Persist routine notifications so the next user reply can see them as context.

CREATE TABLE conversation_notifications (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    conversation_scope_id TEXT,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    content TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consumed_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ
);

CREATE INDEX idx_conversation_notifications_lookup
    ON conversation_notifications(user_id, channel, conversation_scope_id, consumed_at, created_at);
