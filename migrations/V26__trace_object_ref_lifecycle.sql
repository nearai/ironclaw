ALTER TABLE trace_object_refs
    ADD COLUMN IF NOT EXISTS invalidated_at TIMESTAMPTZ;

ALTER TABLE trace_object_refs
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

ALTER TABLE trace_object_refs
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE INDEX IF NOT EXISTS idx_trace_object_refs_lifecycle
    ON trace_object_refs (tenant_id, submission_id, invalidated_at, deleted_at);
