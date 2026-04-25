CREATE TABLE IF NOT EXISTS trace_vector_entries (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID NOT NULL,
    vector_entry_id UUID NOT NULL,
    vector_store TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    embedding_dimension INTEGER NOT NULL CHECK (embedding_dimension > 0),
    embedding_version TEXT NOT NULL,
    source_projection TEXT NOT NULL CHECK (source_projection IN ('canonical_summary', 'redacted_messages', 'redacted_tool_sequence')),
    source_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'invalidated', 'deleted')),
    nearest_trace_ids TEXT[] NOT NULL DEFAULT '{}',
    cluster_id TEXT,
    duplicate_score REAL,
    novelty_score REAL,
    indexed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id, vector_entry_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, derived_id)
        REFERENCES trace_derived_records (tenant_id, derived_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_vector_entries_source
    ON trace_vector_entries (tenant_id, submission_id, status);
CREATE INDEX IF NOT EXISTS idx_trace_vector_entries_cluster
    ON trace_vector_entries (tenant_id, cluster_id, status)
    WHERE cluster_id IS NOT NULL;
