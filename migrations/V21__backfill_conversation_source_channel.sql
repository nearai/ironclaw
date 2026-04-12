-- Backfill source_channel for conversations created before V15.
--
-- V15 added the source_channel column as nullable. Pre-V15 rows therefore
-- have NULL source_channel after the upgrade, and the runtime approval
-- check (see is_approval_authorized in src/agent/session.rs) is fail-closed
-- on None — so any legacy conversation rehydrated from DB after restart
-- would reject every approval message, including from its own original
-- channel.
--
-- The original creating channel is recorded in the `channel` column, so
-- the safe backfill is `source_channel = channel` for any row whose
-- source_channel was never populated.
UPDATE conversations
SET source_channel = channel
WHERE source_channel IS NULL;
