-- Backfill source_channel for conversations created before V15.
-- The owning channel is the closest durable approximation of the original
-- creator channel, and preserves approval authorization after hydration.
UPDATE conversations
SET source_channel = channel
WHERE source_channel IS NULL;
