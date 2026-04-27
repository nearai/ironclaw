CREATE TABLE trace_tenant_access_grants (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    grant_id UUID NOT NULL,
    principal_ref TEXT NOT NULL,
    role TEXT NOT NULL CHECK (
        role IN (
            'contributor',
            'reviewer',
            'admin',
            'export_worker',
            'retention_worker',
            'vector_worker',
            'benchmark_worker',
            'utility_worker',
            'process_eval_worker',
            'revocation_worker'
        )
    ),
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    allowed_consent_scopes JSONB NOT NULL DEFAULT '[]',
    allowed_uses JSONB NOT NULL DEFAULT '[]',
    issuer TEXT,
    audience TEXT,
    subject TEXT,
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by_principal_ref TEXT,
    revoked_by_principal_ref TEXT,
    reason TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, grant_id)
);

CREATE INDEX idx_trace_tenant_access_grants_principal
    ON trace_tenant_access_grants (tenant_id, principal_ref, status, expires_at);
CREATE INDEX idx_trace_tenant_access_grants_role
    ON trace_tenant_access_grants (tenant_id, role, status, expires_at);
CREATE INDEX idx_trace_tenant_access_grants_issuer_subject
    ON trace_tenant_access_grants (tenant_id, issuer, subject)
    WHERE issuer IS NOT NULL OR subject IS NOT NULL;

ALTER TABLE trace_tenant_access_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenant_access_grants
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
