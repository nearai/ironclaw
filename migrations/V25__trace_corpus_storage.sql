-- Trace Commons corpus metadata storage.
--
-- The private corpus never stores raw local traces centrally. These tables
-- track tenant-scoped redacted submissions, derived artifacts, audit events,
-- credit ledger entries, and revocation tombstones.

CREATE TABLE IF NOT EXISTS trace_tenants (
    tenant_id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS trace_submissions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    auth_principal_ref TEXT NOT NULL,
    contributor_pseudonym TEXT,
    submitted_tenant_scope_ref TEXT,
    schema_version TEXT NOT NULL,
    consent_policy_version TEXT NOT NULL,
    consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    retention_policy_id TEXT NOT NULL,
    status TEXT NOT NULL,
    privacy_risk TEXT NOT NULL,
    redaction_pipeline_version TEXT NOT NULL,
    redaction_hash TEXT NOT NULL,
    canonical_summary_hash TEXT,
    submission_score REAL,
    credit_points_pending REAL,
    credit_points_final REAL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    purged_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, submission_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_submissions_tenant_status
    ON trace_submissions (tenant_id, status, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_trace_id
    ON trace_submissions (tenant_id, trace_id);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_contributor
    ON trace_submissions (tenant_id, contributor_pseudonym)
    WHERE contributor_pseudonym IS NOT NULL;

CREATE TABLE IF NOT EXISTS trace_object_refs (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    object_ref_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL,
    object_store TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    encryption_key_ref TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    compression TEXT,
    created_by_job_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_object_refs_kind
    ON trace_object_refs (tenant_id, artifact_kind, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_derived_records (
    tenant_id TEXT NOT NULL,
    derived_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    status TEXT NOT NULL,
    worker_kind TEXT NOT NULL,
    worker_version TEXT NOT NULL,
    input_object_ref_id UUID,
    input_hash TEXT NOT NULL,
    output_object_ref_id UUID,
    canonical_summary TEXT,
    canonical_summary_hash TEXT,
    task_success TEXT,
    privacy_risk TEXT,
    event_count INTEGER,
    duplicate_score REAL,
    novelty_score REAL,
    cluster_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, input_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, output_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_derived_records_submission
    ON trace_derived_records (tenant_id, submission_id, worker_kind);
CREATE INDEX IF NOT EXISTS idx_trace_derived_records_cluster
    ON trace_derived_records (tenant_id, cluster_id)
    WHERE cluster_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS trace_audit_events (
    tenant_id TEXT NOT NULL,
    audit_event_id UUID NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT,
    request_id TEXT,
    submission_id UUID,
    object_ref_id UUID,
    export_manifest_id UUID,
    decision_inputs_hash TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, audit_event_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_audit_events_submission
    ON trace_audit_events (tenant_id, submission_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_audit_events_action
    ON trace_audit_events (tenant_id, action, occurred_at DESC);

CREATE TABLE IF NOT EXISTS trace_credit_ledger (
    tenant_id TEXT NOT NULL,
    credit_event_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    credit_account_ref TEXT NOT NULL,
    event_type TEXT NOT NULL,
    points_delta TEXT NOT NULL,
    reason TEXT NOT NULL,
    external_ref TEXT,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    settlement_state TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, credit_event_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_credit_ledger_account
    ON trace_credit_ledger (tenant_id, credit_account_ref, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_credit_ledger_submission
    ON trace_credit_ledger (tenant_id, submission_id, occurred_at DESC);

CREATE TABLE IF NOT EXISTS trace_tombstones (
    tenant_id TEXT NOT NULL,
    tombstone_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID,
    redaction_hash TEXT,
    canonical_summary_hash TEXT,
    reason TEXT NOT NULL,
    effective_at TIMESTAMPTZ NOT NULL,
    retain_until TIMESTAMPTZ,
    created_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, tombstone_id),
    UNIQUE (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_tombstones_effective
    ON trace_tombstones (tenant_id, effective_at DESC);
