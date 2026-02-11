-- Rename claude_code_events to job_events (generic for all sandbox job types).
ALTER TABLE claude_code_events RENAME TO job_events;
ALTER INDEX idx_cc_events_job RENAME TO idx_job_events_job;
