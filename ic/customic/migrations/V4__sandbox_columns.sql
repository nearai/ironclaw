-- Add project_dir and user_id columns for sandbox job tracking.
-- user_id was previously hardcoded to "default" in the Rust layer;
-- now it's persisted so we can filter per-user.

ALTER TABLE agent_jobs ADD COLUMN IF NOT EXISTS project_dir TEXT;
ALTER TABLE agent_jobs ADD COLUMN IF NOT EXISTS user_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_agent_jobs_source ON agent_jobs(source);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_user ON agent_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_created ON agent_jobs(created_at DESC);
