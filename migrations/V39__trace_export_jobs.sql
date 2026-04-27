CREATE TABLE trace_export_access_grants (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_job_id UUID NOT NULL,
    grant_id UUID NOT NULL,
    caller_principal_ref TEXT NOT NULL,
    requested_dataset_kind TEXT NOT NULL,
    purpose TEXT NOT NULL,
    max_item_cap INTEGER CHECK (max_item_cap IS NULL OR max_item_cap >= 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'consumed', 'revoked', 'expired')),
    requested_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    metadata_json JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, grant_id),
    UNIQUE (tenant_id, export_job_id, grant_id)
);

CREATE INDEX idx_trace_export_access_grants_job
    ON trace_export_access_grants (tenant_id, export_job_id);
CREATE INDEX idx_trace_export_access_grants_principal
    ON trace_export_access_grants (tenant_id, caller_principal_ref, expires_at);
CREATE INDEX idx_trace_export_access_grants_status
    ON trace_export_access_grants (tenant_id, status, expires_at);

CREATE TABLE trace_export_jobs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_job_id UUID NOT NULL,
    grant_id UUID NOT NULL,
    caller_principal_ref TEXT NOT NULL,
    requested_dataset_kind TEXT NOT NULL,
    purpose TEXT NOT NULL,
    max_item_cap INTEGER CHECK (max_item_cap IS NULL OR max_item_cap >= 0),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'complete', 'failed', 'cancelled', 'expired')),
    requested_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    result_manifest_id UUID,
    item_count INTEGER CHECK (item_count IS NULL OR item_count >= 0),
    last_error TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, export_job_id),
    FOREIGN KEY (tenant_id, grant_id)
        REFERENCES trace_export_access_grants (tenant_id, grant_id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_trace_export_jobs_requested
    ON trace_export_jobs (tenant_id, requested_at DESC);
CREATE INDEX idx_trace_export_jobs_status
    ON trace_export_jobs (tenant_id, status, updated_at DESC);
CREATE INDEX idx_trace_export_jobs_grant
    ON trace_export_jobs (tenant_id, grant_id);

ALTER TABLE trace_export_access_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_access_grants
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_export_jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_jobs
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
