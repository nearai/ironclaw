-- Add retry_attempts column to job_actions for tracking tool-level retries.
ALTER TABLE job_actions ADD COLUMN retry_attempts INTEGER NOT NULL DEFAULT 0;
