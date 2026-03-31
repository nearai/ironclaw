-- Add source_channel to conversations for cross-channel approval authorization.
-- Tracks which channel originally created a conversation so that approval
-- messages from other channels can be validated.
ALTER TABLE conversations ADD COLUMN source_channel TEXT;
