CREATE TABLE trace_revocation_propagation_items (
    tenant_id TEXT NOT NULL,
    propagation_item_id UUID NOT NULL,
    source_submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    target_kind TEXT NOT NULL CHECK (
        target_kind IN (
            'object_ref',
            'export_manifest',
            'export_manifest_item',
            'vector_entry',
            'derived_record',
            'benchmark_artifact',
            'ranker_artifact',
            'credit_settlement',
            'physical_delete_receipt'
        )
    ),
    target_json JSONB NOT NULL,
    action TEXT NOT NULL CHECK (
        action IN (
            'invalidate_metadata',
            'invalidate_export_membership',
            'invalidate_vector',
            'invalidate_benchmark_artifact',
            'invalidate_ranker_artifact',
            'reverse_credit_settlement',
            'delete_object_payload',
            'record_physical_delete_receipt'
        )
    ),
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'in_progress', 'done', 'failed', 'skipped')
    ),
    idempotency_key TEXT NOT NULL,
    reason TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error TEXT,
    next_attempt_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    evidence_hash TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, propagation_item_id),
    UNIQUE (tenant_id, idempotency_key),
    FOREIGN KEY (tenant_id, source_submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_revocation_propagation_source
    ON trace_revocation_propagation_items (tenant_id, source_submission_id, created_at ASC);
CREATE INDEX idx_trace_revocation_propagation_due
    ON trace_revocation_propagation_items (tenant_id, status, next_attempt_at, created_at ASC);
CREATE INDEX idx_trace_revocation_propagation_target
    ON trace_revocation_propagation_items (tenant_id, target_kind, updated_at DESC);

ALTER TABLE trace_revocation_propagation_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_revocation_propagation_items
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
