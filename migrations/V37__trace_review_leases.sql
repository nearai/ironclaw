ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS review_assigned_to_principal_ref TEXT;

ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS review_assigned_at TIMESTAMPTZ;

ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS review_lease_expires_at TIMESTAMPTZ;

ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS review_due_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_trace_submissions_review_lease
    ON trace_submissions (tenant_id, status, review_lease_expires_at, received_at DESC);
