-- Partial unique indexes to prevent duplicate singleton conversations.
-- These guard against TOCTOU races in get_or_create_routine_conversation
-- and get_or_create_heartbeat_conversation.

-- One routine conversation per user per routine_id.
CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_routine
ON conversations (user_id, (metadata->>'routine_id'))
WHERE metadata->>'routine_id' IS NOT NULL;

-- One heartbeat conversation per user.
CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_heartbeat
ON conversations (user_id)
WHERE metadata->>'thread_type' = 'heartbeat';
