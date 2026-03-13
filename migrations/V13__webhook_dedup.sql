-- Webhook message deduplication table
-- Prevents duplicate processing when channels retry on errors

CREATE TABLE IF NOT EXISTS webhook_message_dedup (
    -- Composite key: channel name + message ID from the channel
    -- e.g., "whatsapp:wamid.HBgM..." or "telegram:12345"
    key TEXT PRIMARY KEY,

    -- When this message was first seen
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup queries (delete old records)
CREATE INDEX IF NOT EXISTS idx_webhook_dedup_created_at
    ON webhook_message_dedup(created_at);

-- Comment explaining purpose
COMMENT ON TABLE webhook_message_dedup IS
    'Deduplication table for webhook messages. Channels like WhatsApp retry on 5xx for up to 7 days. This table ensures idempotent processing.';
