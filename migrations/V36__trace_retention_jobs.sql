CREATE TABLE trace_retention_jobs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    retention_job_id UUID NOT NULL,
    purpose TEXT NOT NULL,
    dry_run BOOLEAN NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'running', 'dry_run', 'complete', 'failed', 'paused')),
    requested_by_principal_ref TEXT NOT NULL,
    requested_by_role TEXT NOT NULL,
    purge_expired_before TIMESTAMPTZ,
    prune_export_cache BOOLEAN NOT NULL DEFAULT TRUE,
    max_export_age_hours BIGINT,
    audit_event_id UUID,
    action_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    selected_revoked_count INTEGER NOT NULL DEFAULT 0 CHECK (selected_revoked_count >= 0),
    selected_expired_count INTEGER NOT NULL DEFAULT 0 CHECK (selected_expired_count >= 0),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, retention_job_id)
);

CREATE INDEX idx_trace_retention_jobs_created
    ON trace_retention_jobs (tenant_id, created_at DESC);
CREATE INDEX idx_trace_retention_jobs_status
    ON trace_retention_jobs (tenant_id, status, updated_at DESC);

CREATE TABLE trace_retention_job_items (
    tenant_id TEXT NOT NULL,
    retention_job_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('revoke', 'expire', 'purge')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'done', 'failed', 'skipped')),
    reason TEXT NOT NULL,
    action_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, retention_job_id, submission_id, action),
    FOREIGN KEY (tenant_id, retention_job_id)
        REFERENCES trace_retention_jobs (tenant_id, retention_job_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_retention_job_items_submission
    ON trace_retention_job_items (tenant_id, submission_id, created_at DESC);

ALTER TABLE trace_retention_jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_retention_jobs
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_retention_job_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_retention_job_items
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
