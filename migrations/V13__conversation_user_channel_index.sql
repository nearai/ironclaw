-- Composite index for find_conversation_by_user_channel query.
-- Optimizes the lookup of the most recent conversation for a user on a channel.
CREATE INDEX IF NOT EXISTS idx_conversations_user_channel_activity
ON conversations(user_id, channel, last_activity DESC);
