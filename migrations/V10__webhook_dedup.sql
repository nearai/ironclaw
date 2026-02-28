-- Webhook message deduplication table
-- Tracks which webhook messages have been processed to prevent duplicates
-- when WhatsApp retries after a 500 response

CREATE TABLE webhook_message_dedup (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel TEXT NOT NULL,
    external_message_id TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(channel, external_message_id)
);

-- Index for fast lookup by channel + message_id
CREATE INDEX idx_webhook_dedup_channel_msg ON webhook_message_dedup(channel, external_message_id);

-- Auto-cleanup: delete entries older than 7 days (WhatsApp max retry window)
-- This keeps the table small while covering all retry scenarios
CREATE OR REPLACE FUNCTION cleanup_old_dedup_entries() RETURNS void AS $$
BEGIN
    DELETE FROM webhook_message_dedup WHERE processed_at < NOW() - INTERVAL '7 days';
END;
$$ LANGUAGE plpgsql;
