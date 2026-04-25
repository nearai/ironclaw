-- Tenant-scoped Trace Commons contribution policy.
--
-- The ingest service can already enforce policy from local env config. This
-- table gives production deployments a durable tenant-scoped policy record for
-- allowed consent scopes and allowed trace-card uses.

CREATE TABLE trace_tenant_policies (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    policy_version TEXT NOT NULL,
    allowed_consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    updated_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE trace_tenant_policies ENABLE ROW LEVEL SECURITY;

CREATE POLICY trace_corpus_tenant_isolation ON trace_tenant_policies
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
