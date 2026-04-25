CREATE TABLE trace_export_manifest_items (
    tenant_id TEXT NOT NULL,
    export_manifest_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    derived_id UUID,
    object_ref_id UUID,
    vector_entry_id UUID,
    source_status_at_export TEXT NOT NULL,
    source_hash_at_export TEXT NOT NULL,
    source_invalidated_at TIMESTAMPTZ,
    source_invalidation_reason TEXT CHECK (
        source_invalidation_reason IS NULL
        OR source_invalidation_reason IN ('revoked', 'expired', 'purged')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, export_manifest_id, submission_id),
    FOREIGN KEY (tenant_id, export_manifest_id)
        REFERENCES trace_export_manifests (tenant_id, export_manifest_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_export_manifest_items_source
    ON trace_export_manifest_items (tenant_id, submission_id, source_invalidated_at);
CREATE INDEX idx_trace_export_manifest_items_manifest
    ON trace_export_manifest_items (tenant_id, export_manifest_id, created_at ASC);
