-- Add agent review columns to routines table.
-- When enabled per-routine + globally, completed routine results are injected
-- into the agent loop so the agent can interpret and relay them to the user.

ALTER TABLE routines ADD COLUMN agent_review_on_success BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE routines ADD COLUMN agent_review_on_failure BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE routines ADD COLUMN agent_review_on_attention BOOLEAN NOT NULL DEFAULT false;
