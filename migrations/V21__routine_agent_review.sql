-- Per-routine agent review: inject completion results into the agentic loop.
ALTER TABLE routines ADD COLUMN agent_review_on_success BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE routines ADD COLUMN agent_review_on_failure BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE routines ADD COLUMN agent_review_on_attention BOOLEAN NOT NULL DEFAULT false;
