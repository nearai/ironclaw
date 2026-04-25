CREATE TABLE IF NOT EXISTS trace_export_manifests (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_manifest_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL,
    purpose_code TEXT,
    audit_event_id UUID,
    source_submission_ids UUID[] NOT NULL DEFAULT '{}',
    source_submission_ids_hash TEXT NOT NULL,
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    generated_at TIMESTAMPTZ NOT NULL,
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, export_manifest_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_export_manifests_generated
    ON trace_export_manifests (tenant_id, generated_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_export_manifests_hash
    ON trace_export_manifests (tenant_id, source_submission_ids_hash);
