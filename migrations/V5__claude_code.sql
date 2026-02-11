-- Track which mode a sandbox job uses (worker vs claude_code).
ALTER TABLE agent_jobs ADD COLUMN IF NOT EXISTS job_mode TEXT NOT NULL DEFAULT 'worker';

-- Persist Claude Code streaming events so they survive restarts and can be
-- loaded when the frontend opens a job detail view after the fact.
CREATE TABLE IF NOT EXISTS claude_code_events (
    id BIGSERIAL PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES agent_jobs(id),
    event_type TEXT NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_cc_events_job ON claude_code_events(job_id, id);
