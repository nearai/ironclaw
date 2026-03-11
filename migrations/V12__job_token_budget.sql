-- Add token budget tracking columns to agent_jobs.
--
-- Tracks max_tokens (configured limit per job) and total_tokens_used (running total)
-- to enforce job-level token budgets and prevent budget bypass via user-supplied metadata.

ALTER TABLE agent_jobs ADD COLUMN max_tokens BIGINT NOT NULL DEFAULT 0;
ALTER TABLE agent_jobs ADD COLUMN total_tokens_used BIGINT NOT NULL DEFAULT 0;
