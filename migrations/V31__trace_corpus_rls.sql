-- Trace Commons row-level security readiness.
--
-- Policies are intentionally tenant-scoped and narrow to the Trace Commons
-- corpus metadata tables. They rely on callers setting a transaction-local
-- tenant context before accessing rows:
--
--   SELECT set_config('ironclaw.trace_tenant_id', '<tenant-id>', true);
--
-- Do not use session-level SET with pooled PostgreSQL connections; that can
-- leak tenant context across pooled connection reuse. We also intentionally do
-- not FORCE ROW LEVEL SECURITY in this migration. Table owners bypass these
-- policies unless FORCE is enabled, and keeping owner bypass preserves safe
-- migration/backfill/repair behavior while the runtime moves store operations
-- to transaction-local tenant context.

ALTER TABLE trace_tenants ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenants
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_submissions ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_submissions
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_object_refs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_object_refs
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_derived_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_derived_records
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_audit_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_audit_events
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_credit_ledger ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_ledger
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_tombstones ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tombstones
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_vector_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_vector_entries
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_export_manifests ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_manifests
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));

ALTER TABLE trace_export_manifest_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_manifest_items
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
