# Trace Commons Production Storage Plan

This document tracks the migration path from the file-backed Trace Commons ingest MVP to production storage. It assumes the existing `ironclaw.trace_contribution.v1` envelope remains the upload contract, while production ingest moves trust, authorization, retention, review, export, and credit accounting into durable services.

## Current State

The MVP ingestion service still serves tenant-scoped JSON files under `TRACE_COMMONS_DATA_DIR` and derives lightweight records for review, analytics, credit, and replay export. That remains appropriate for local development and controlled pilots.

This branch now contains the first production-storage bridge:

- `migrations/V25__trace_corpus_storage.sql`, `migrations/V26__trace_object_ref_lifecycle.sql`, `migrations/V27__trace_corpus_rich_metadata.sql`, `migrations/V28__trace_vector_entries.sql`, `migrations/V29__trace_export_manifests.sql`, `migrations/V30__trace_export_manifest_items.sql`, `migrations/V32__trace_audit_hash_chain.sql`, `migrations/V33__trace_tenant_policies.sql`, and matching libSQL incremental migrations.
- `src/trace_corpus_storage.rs` and `TraceCorpusStore` implementations for PostgreSQL and libSQL.
- Optional ingest-service DB dual-write behind `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- Optional DB-backed tenant policy reads behind `TRACE_COMMONS_DB_TENANT_POLICY_READS=true`.
- Tenant-policy export ABAC for replay, benchmark, and ranker exports using existing allowed consent scopes and allowed uses.
- Optional fail-closed benchmark/ranker source object-ref validation behind `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true`.
- Admin-token tenant policy management through `/v1/admin/tenant-policy`, with hash-chained file audit events and safe DB audit metadata for policy version, allow-list counts, and the policy projection hash.
- Admin-token config inspection through `/v1/admin/config-status`, returning only safe schema, DB cutover, object-primary, guardrail, max export item cap, submission quota, legal hold, and object-store-provider status fields with a read audit event.
- Optional encrypted local artifact storage behind `TRACE_COMMONS_ARTIFACT_KEY_HEX`, with `TRACE_COMMONS_OBJECT_STORE=local_service` selecting the service-owned local encrypted backend used for production-shaped object refs.
- Optional object-primary submit/review mode behind `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true`, which requires the DB/object-ref cutover guards and skips plaintext submitted/reviewed envelope body files while retaining compatibility metadata, derived records, and file audit rows.
- Optional object-primary replay export mode behind `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true`, which requires DB replay selection, required replay object refs, required DB mirror writes, and the service-local encrypted object store.
- Optional object-primary benchmark/ranker export mode behind `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true`, which requires DB reviewer reads, required source object refs, export guardrails, required DB mirror writes, and the service-local encrypted object store before skipping plaintext benchmark artifact and ranker provenance files.
- Optional legal-hold retention policy IDs behind `TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES`, preventing maintenance from newly expiring or purging matching policy classes.
- Optional fail-closed maintenance promotion gate behind `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true`, which requires DB dual-write, rejects maintenance requests that omit `reconcile_db_mirror: true`, exposes compact `blocking_gaps`, and turns DB/file reconciliation gaps into `409 Conflict` maintenance failures.
- Caller-level tests for tenant-scoped writes, DB-backed tenant policy enforcement, review/revocation state, delayed credit events, encrypted artifact receipts, and DB object-ref replay reads through the service-owned local object-store backend.

Production still needs stronger guarantees before broad tenant rollout:

- Relational metadata for broader workflow state, credit, audit, retention, and export manifests.
- Encrypted object storage for redacted envelope bodies and large derived artifacts. The current service-owned local backend is a migration step toward remote object stores: it records a stable object-store provider alias in `trace_object_refs`, stores ciphertext under tenant-hashed paths, verifies ciphertext hashes on read, and lets replay exports fail closed when active object refs are required.
- A vector store for approved redacted summaries and allowed redacted trace fields.
- Tenant isolation derived from authenticated request identity, never from envelope fields.
- Idempotent revocation and retention propagation across metadata, objects, vectors, worker queues, exports, and credit.

The production migration should not replace the local queue/capture semantics. Clients should keep producing locally redacted envelopes and the service should keep re-scrubbing before acceptance.

## Storage Boundaries

Use the relational database for metadata, authorization decisions, workflow state, hashes, object references, indexes, and append-only ledgers. Do not store full trace bodies, large benchmark payloads, vector embeddings, or export blobs directly in relational rows unless the deployment is a small libSQL-only single-node pilot.

Use encrypted object storage for:

- Submitted redacted envelopes.
- Server re-scrubbed envelope versions.
- Review snapshots when reviewers need a frozen view.
- Benchmark/replay artifacts.
- Export result files and manifest payloads when they exceed comfortable row size.
- Worker intermediate artifacts that must survive restarts.

Use the vector database for:

- Embeddings generated only from approved redacted summaries or explicitly allowed redacted fields.
- Tenant-scoped vector ids linked back to submission ids and derived record ids.
- Duplicate, novelty, nearest-neighbor, and cluster metadata. Persist the final worker output in relational metadata so the vector index can be rebuilt.

Do not put bearer tokens, raw local paths, raw sidecar spans, unredacted trace text, or raw tool payloads into any production store.

## Schema Sketch

## Concrete DB Migration Slice

This first production-storage slice has now been implemented as a dark-launch bridge. It creates the relational control plane only: envelope payloads belong in encrypted artifact storage, and vector payloads can stay in a vector store or backend-specific index. `src/bin/trace_commons_ingest.rs` can mirror metadata into the DB when `TRACE_COMMONS_DB_DUAL_WRITE=true`, including submission redaction counts, derived summary/tool/coverage metadata, vector-entry metadata, replay export manifest metadata, replay export source item rows, benchmark/ranker export provenance metadata, tombstones, and read/export/credit audit events. Vector indexing computes deterministic redacted-summary similarity for nearest-neighbor metadata, keeping exact canonical-summary hash matches as the strongest duplicate signal. When encrypted artifact storage is configured, vector indexing now also writes a redacted canonical-summary vector payload as a `worker_intermediate` object ref so a later embedding worker has an auditable, tenant-scoped payload handle without adding a DB column. Export audit rows now carry deterministic source-list hashes in `decision_inputs_hash` for replay datasets, benchmark conversion artifacts, and ranker training exports; file-backed audit rows also carry optional `previous_event_hash`/`event_hash` fields so pilot logs can be checked for simple append-order tampering while legacy rows remain readable, and DB audit rows mirror those chain fields when they are present on the file-backed event. Maintenance can return a verifier report with `verify_audit_chain: true`. Replay dataset exports also mirror durable tenant-scoped manifest rows with source ids, source-list hashes, per-source status/hash snapshots, and the active submitted-envelope object ref used at export time. Benchmark and ranker exports write file-backed provenance manifests by default and opportunistically mirror them into the same export manifest/item tables with source derived artifact refs plus active canonical-summary vector entry refs when vector metadata has already been indexed, while the replay manifest listing endpoint filters DB rows back to replay dataset manifests. Tenant policies now gate replay, benchmark, and ranker export requests and source selection using the same allowed-scope/allowed-use columns as ingest. `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true` requires DB dual-write and makes benchmark/ranker exports fail closed before artifact, provenance, or utility-credit publication when any selected source lacks an active submitted-envelope object ref that can be tenant/hash verified. `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true` requires DB dual-write, required DB mirror writes, DB reviewer reads, reviewer object-ref reads, and the service-local encrypted object store, then writes submitted and reviewed envelope bodies only to the object store while leaving compatibility metadata/derived/audit files in place. `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true` requires DB dual-write, required DB mirror writes, DB replay export reads, replay object-ref-required reads, and the service-local encrypted object store, then keeps replay export body reads on active DB object refs without file fallback. `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true` requires DB dual-write, required DB mirror writes, DB reviewer reads, required derived source object refs, export guardrails, and the service-local encrypted object store, then skips plaintext benchmark artifact/provenance and ranker provenance files; DB manifest/items remain the purpose-filter and lifecycle-invalidation index. The maintenance endpoint can expire past-due pilot records, mirror expiration status plus artifact invalidation into the DB, invalidate benchmark/ranker provenance manifests, backfill pilot file records into the DB, index accepted canonical summaries into deterministic vector metadata rows with `index_vectors: true`, and return a file-vs-DB reconciliation report with `reconcile_db_mirror: true`. Backfill isolates per-submission, credit-event, audit-event, and replay-manifest failures, returns `db_mirror_backfill_failed` plus bounded failure details, and keeps valid records moving while DB setup/listing failures still fail fast. Reconciliation now includes submission, derived, object-ref, vector, credit-ledger, audit-event, replay/export-manifest, export-item, revocation/tombstone counts, active derived/export rows that still point at invalid sources, reader-projection parity for contributor credit, reviewer metadata, analytics, audit, and replay/export manifest surfaces, plus compact `blocking_gaps`; `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` requires DB dual-write, rejects maintenance requests that omit `reconcile_db_mirror`, and turns those promotion-blocking gaps into `409 Conflict` maintenance failures after the normal maintenance audit event is appended. Reconciliation without a configured DB mirror returns `503 Service Unavailable`. File-backed APIs remain the default source of pilot responses. `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true` can switch contributor credit, credit-event, and submission-status reads to the DB mirror after dual-write or backfill is in place. `TRACE_COMMONS_DB_REVIEWER_READS=true` can switch reviewer/admin metadata reads for analytics, trace listing, quarantine queue, active-learning queue, benchmark candidate conversion, and ranker exports to the DB mirror; review decisions also prefer active DB object refs for submitted-envelope body reads, mirror a content-read audit row, append a fresh reviewed-envelope object ref after approval or rejection, and can fail closed with `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true` when no active object ref exists. `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true` can select replay export records from DB metadata and resolve submitted envelope bodies through active DB object refs for file or encrypted local artifact stores, with tenant/object-ref/hash verification and content-read audit mirroring that records `object_ref_id` for DB object-ref reads. Compatibility mode falls back to the file-backed envelope body if no active DB object ref exists; `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true` makes that surface fail closed. `TRACE_COMMONS_DB_AUDIT_READS=true` can serve reviewer audit reads from the DB mirror. `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` is the write-side production cutover switch: it requires DB dual-write and makes critical mirror misses on submissions, revocations, reviews, credit, exports/provenance, and audit/content-read rows fail closed.

### Safe Migration Naming

Historical local state when this slice was created:

- Local PostgreSQL migrations in this worktree end at `migrations/V23__list_workspace_files_escape_like.sql`.
- Local `src/db/libsql_migrations.rs` ends at incremental version `22`.
- The local `origin/staging` ref already contains `migrations/V24__llm_calls_created_at_index.sql` and libSQL incremental version `24`.

Completed guidance:

- Before creating the migration, refresh refs with `git fetch origin` and re-check:
  - `git ls-tree --name-only origin/staging:migrations`
  - `git ls-tree --name-only origin/main:migrations`
  - `git show origin/staging:src/db/libsql_migrations.rs | rg '"[a-z_]+",|\\([[:space:]]*[0-9]+,'`
  - `git show origin/main:src/db/libsql_migrations.rs | rg '"[a-z_]+",|\\([[:space:]]*[0-9]+,'`
- Staging/main were highest at `V24`, so the PostgreSQL migration is `migrations/V25__trace_corpus_storage.sql`.
- The libSQL migration is incremental version `25` named `"trace_corpus_storage"`. The libSQL number does not always match PostgreSQL historically, but both staging and this batch are now highest at `25`.
- Do not add `IF NOT EXISTS` to PostgreSQL migration DDL unless the repo's refinery policy changes. PostgreSQL migrations should be one-shot and checksum-stable.
- For libSQL, use `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`, and add only idempotent DDL to `INCREMENTAL_MIGRATIONS`.
- After adding a real PostgreSQL migration, update `migrations/checksums.lock` using the repo's migration checksum workflow.

### PostgreSQL DDL Sketch

```sql
CREATE TABLE trace_tenants (
    tenant_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended', 'retention_only', 'deleted')),
    data_residency_region TEXT,
    default_retention_policy_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE trace_tenant_policies (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    policy_version TEXT NOT NULL,
    allowed_consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    updated_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE trace_access_grants (
    grant_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    principal_ref TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('contributor', 'reviewer', 'admin', 'export_worker', 'retention_worker', 'vector_worker', 'benchmark_worker', 'process_eval_worker')),
    allowed_scopes TEXT[] NOT NULL DEFAULT '{}',
    allowed_uses TEXT[] NOT NULL DEFAULT '{}',
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by TEXT,
    revoked_by TEXT,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, principal_ref, role)
);
CREATE INDEX idx_trace_access_grants_principal ON trace_access_grants(tenant_id, principal_ref);

CREATE TABLE trace_submissions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    auth_principal_ref TEXT NOT NULL,
    contributor_pseudonym TEXT,
    submitted_tenant_scope_ref TEXT,
    schema_version TEXT NOT NULL,
    consent_policy_version TEXT NOT NULL,
    consent_scopes TEXT[] NOT NULL,
    allowed_uses TEXT[] NOT NULL,
    retention_policy_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('received', 'accepted', 'quarantined', 'rejected', 'revoked', 'expired', 'purged')),
    privacy_risk TEXT NOT NULL CHECK (privacy_risk IN ('low', 'medium', 'high')),
    redaction_pipeline_version TEXT NOT NULL,
    redaction_hash TEXT NOT NULL,
    canonical_summary_hash TEXT,
    submission_score REAL,
    credit_points_pending NUMERIC(18, 6),
    credit_points_final NUMERIC(18, 6),
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    purged_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id)
);
CREATE INDEX idx_trace_submissions_status_received ON trace_submissions(tenant_id, status, received_at DESC);
CREATE INDEX idx_trace_submissions_summary_hash ON trace_submissions(tenant_id, canonical_summary_hash);
CREATE INDEX idx_trace_submissions_expires ON trace_submissions(tenant_id, expires_at);
CREATE INDEX idx_trace_submissions_contributor ON trace_submissions(tenant_id, contributor_pseudonym);

CREATE TABLE trace_object_refs (
    object_ref_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN ('submitted_envelope', 'rescrubbed_envelope', 'review_snapshot', 'benchmark_artifact', 'export_artifact', 'worker_intermediate')),
    object_store TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    encryption_key_ref TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    compression TEXT,
    created_by_job_id UUID,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    UNIQUE (tenant_id, object_ref_id),
    UNIQUE (tenant_id, submission_id, object_ref_id),
    UNIQUE (tenant_id, object_store, object_key)
);
CREATE INDEX idx_trace_object_refs_submission ON trace_object_refs(tenant_id, submission_id, artifact_kind);

CREATE TABLE trace_derived_records (
    derived_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('current', 'invalidated', 'superseded', 'revoked', 'expired')),
    worker_kind TEXT NOT NULL CHECK (worker_kind IN ('server_rescrub', 'summary', 'duplicate_precheck', 'embedding', 'ranking', 'benchmark_conversion', 'process_evaluation')),
    worker_version TEXT NOT NULL,
    input_object_ref_id UUID,
    input_hash TEXT NOT NULL,
    output_object_ref_id UUID,
    canonical_summary TEXT,
    canonical_summary_hash TEXT,
    task_success TEXT,
    privacy_risk TEXT CHECK (privacy_risk IS NULL OR privacy_risk IN ('low', 'medium', 'high')),
    event_count INTEGER,
    tool_sequence TEXT[] NOT NULL DEFAULT '{}',
    tool_categories TEXT[] NOT NULL DEFAULT '{}',
    coverage_tags TEXT[] NOT NULL DEFAULT '{}',
    duplicate_score REAL,
    novelty_score REAL,
    cluster_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, input_object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, output_object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    UNIQUE (tenant_id, submission_id, derived_id)
);
CREATE INDEX idx_trace_derived_current ON trace_derived_records(tenant_id, submission_id, status, worker_kind);
CREATE INDEX idx_trace_derived_summary_hash ON trace_derived_records(tenant_id, canonical_summary_hash);

CREATE TABLE trace_vector_entries (
    vector_entry_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID NOT NULL,
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
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id) ON DELETE CASCADE,
    UNIQUE (tenant_id, submission_id, vector_entry_id)
);
CREATE INDEX idx_trace_vector_entries_source ON trace_vector_entries(tenant_id, submission_id, status);

CREATE TABLE trace_audit_events (
    audit_event_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    job_id UUID,
    submission_id UUID,
    object_ref_id UUID,
    export_manifest_id UUID,
    action TEXT NOT NULL CHECK (action IN ('submit', 'read', 'review', 'credit_mutate', 'revoke', 'export', 'retain', 'purge', 'vector_index', 'benchmark_convert', 'process_evaluate')),
    reason TEXT,
    request_id TEXT,
    decision_inputs_hash TEXT,
    metadata_kind TEXT NOT NULL DEFAULT 'empty' CHECK (metadata_kind IN ('empty', 'submission', 'review_decision', 'export', 'maintenance')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    previous_event_hash TEXT,
    event_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id)
);
CREATE INDEX idx_trace_audit_events_target ON trace_audit_events(tenant_id, submission_id, created_at DESC);
CREATE INDEX idx_trace_audit_events_action ON trace_audit_events(tenant_id, action, created_at DESC);

CREATE TABLE trace_credit_ledger (
    credit_event_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    credit_account_ref TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('accepted', 'privacy_rejection', 'duplicate_rejection', 'benchmark_conversion', 'regression_catch', 'training_utility', 'reviewer_bonus', 'abuse_penalty')),
    points_delta NUMERIC(18, 6) NOT NULL,
    reason TEXT NOT NULL,
    external_ref TEXT,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    settlement_state TEXT NOT NULL CHECK (settlement_state IN ('pending', 'final', 'reversed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE
);
CREATE INDEX idx_trace_credit_ledger_account ON trace_credit_ledger(tenant_id, credit_account_ref, created_at DESC);
CREATE INDEX idx_trace_credit_ledger_submission ON trace_credit_ledger(tenant_id, submission_id);

CREATE TABLE trace_export_manifests (
    export_manifest_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    requested_by_principal_ref TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('replay_dataset', 'benchmark_eval', 'ranking_training', 'model_training', 'analytics')),
    consent_scope_filter TEXT[] NOT NULL DEFAULT '{}',
    allowed_use_filter TEXT[] NOT NULL DEFAULT '{}',
    review_state_filter TEXT[] NOT NULL DEFAULT '{}',
    privacy_risk_filter TEXT[] NOT NULL DEFAULT '{}',
    status TEXT NOT NULL CHECK (status IN ('planned', 'running', 'complete', 'failed', 'revoked_invalid', 'expired_invalid')),
    item_count INTEGER NOT NULL DEFAULT 0,
    manifest_object_ref_id UUID,
    result_object_ref_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    FOREIGN KEY (tenant_id, manifest_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id),
    FOREIGN KEY (tenant_id, result_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id),
    UNIQUE (tenant_id, export_manifest_id)
);

CREATE TABLE trace_export_manifest_items (
    export_manifest_id UUID NOT NULL REFERENCES trace_export_manifests(export_manifest_id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID,
    object_ref_id UUID,
    vector_entry_id UUID,
    source_status_at_export TEXT NOT NULL,
    source_hash_at_export TEXT NOT NULL,
    revoked_after_export_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (export_manifest_id, tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, vector_entry_id) REFERENCES trace_vector_entries(tenant_id, submission_id, vector_entry_id)
);

CREATE TABLE trace_benchmark_artifacts (
    benchmark_artifact_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID,
    benchmark_kind TEXT NOT NULL CHECK (benchmark_kind IN ('replay', 'process_eval', 'regression_case', 'ranking_pair')),
    artifact_version TEXT NOT NULL,
    object_ref_id UUID NOT NULL,
    requirements_hash TEXT,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'approved', 'published', 'invalidated', 'deleted')),
    created_by_job_id UUID,
    published_export_manifest_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, published_export_manifest_id) REFERENCES trace_export_manifests(tenant_id, export_manifest_id)
);

CREATE TABLE trace_tombstones (
    tombstone_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    trace_id UUID,
    redaction_hash TEXT,
    canonical_summary_hash TEXT,
    reason TEXT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_at TIMESTAMPTZ NOT NULL,
    retain_until TIMESTAMPTZ,
    created_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, submission_id)
);
CREATE INDEX idx_trace_tombstones_hashes ON trace_tombstones(tenant_id, redaction_hash, canonical_summary_hash);

CREATE TABLE trace_retention_jobs (
    retention_job_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    cutoff_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'dry_run', 'running', 'complete', 'failed', 'paused')),
    selected_count INTEGER NOT NULL DEFAULT 0,
    purged_count INTEGER NOT NULL DEFAULT 0,
    failed_count INTEGER NOT NULL DEFAULT 0,
    dry_run_report_object_ref_id UUID,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, dry_run_report_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id)
);

CREATE TABLE trace_retention_job_items (
    retention_job_id UUID NOT NULL REFERENCES trace_retention_jobs(retention_job_id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('expire', 'revoke', 'delete_object', 'delete_vector', 'invalidate_export', 'write_tombstone')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'done', 'failed', 'skipped_policy_changed')),
    object_ref_id UUID,
    vector_entry_id UUID,
    export_manifest_id UUID REFERENCES trace_export_manifests(export_manifest_id),
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (retention_job_id, tenant_id, submission_id, action),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, vector_entry_id) REFERENCES trace_vector_entries(tenant_id, submission_id, vector_entry_id)
);
```

V31 adds the first PostgreSQL RLS policy layer for the tenant-scoped Trace Commons metadata tables:

```sql
ALTER TABLE trace_submissions ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_submissions_tenant_isolation ON trace_submissions
    USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('ironclaw.trace_tenant_id', true));
```

The migration intentionally does not use `FORCE ROW LEVEL SECURITY`, so table owners still bypass policies for safe migrations, backfills, and repairs while the runtime moves to transaction-local tenant context. Before production cutover, every PG-backed Trace Commons store path should set `SELECT set_config('ironclaw.trace_tenant_id', $1, true)` inside the operation transaction, and worker roles should get explicit policy variants, not blanket bypass.

### libSQL DDL Sketch

```sql
CREATE TABLE IF NOT EXISTS trace_tenants (
    tenant_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended', 'retention_only', 'deleted')),
    data_residency_region TEXT,
    default_retention_policy_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS trace_tenant_policies (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    policy_version TEXT NOT NULL,
    allowed_consent_scopes TEXT NOT NULL DEFAULT '[]',
    allowed_uses TEXT NOT NULL DEFAULT '[]',
    updated_by_principal_ref TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS trace_access_grants (
    grant_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    principal_ref TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('contributor', 'reviewer', 'admin', 'export_worker', 'retention_worker', 'vector_worker', 'benchmark_worker', 'process_eval_worker')),
    allowed_scopes TEXT NOT NULL DEFAULT '[]',
    allowed_uses TEXT NOT NULL DEFAULT '[]',
    expires_at TEXT,
    revoked_at TEXT,
    created_by TEXT,
    revoked_by TEXT,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (tenant_id, principal_ref, role)
);
CREATE INDEX IF NOT EXISTS idx_trace_access_grants_principal ON trace_access_grants(tenant_id, principal_ref);

CREATE TABLE IF NOT EXISTS trace_submissions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    auth_principal_ref TEXT NOT NULL,
    contributor_pseudonym TEXT,
    submitted_tenant_scope_ref TEXT,
    schema_version TEXT NOT NULL,
    consent_policy_version TEXT NOT NULL,
    consent_scopes TEXT NOT NULL,
    allowed_uses TEXT NOT NULL,
    retention_policy_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('received', 'accepted', 'quarantined', 'rejected', 'revoked', 'expired', 'purged')),
    privacy_risk TEXT NOT NULL CHECK (privacy_risk IN ('low', 'medium', 'high')),
    redaction_pipeline_version TEXT NOT NULL,
    redaction_hash TEXT NOT NULL,
    canonical_summary_hash TEXT,
    submission_score REAL,
    credit_points_pending TEXT,
    credit_points_final TEXT,
    received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    reviewed_at TEXT,
    revoked_at TEXT,
    expires_at TEXT,
    purged_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, submission_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_status_received ON trace_submissions(tenant_id, status, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_summary_hash ON trace_submissions(tenant_id, canonical_summary_hash);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_expires ON trace_submissions(tenant_id, expires_at);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_contributor ON trace_submissions(tenant_id, contributor_pseudonym);

CREATE TABLE IF NOT EXISTS trace_object_refs (
    object_ref_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN ('submitted_envelope', 'rescrubbed_envelope', 'review_snapshot', 'benchmark_artifact', 'export_artifact', 'worker_intermediate')),
    object_store TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    encryption_key_ref TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    compression TEXT,
    created_by_job_id TEXT,
    valid_from TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    invalidated_at TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    UNIQUE (tenant_id, object_ref_id),
    UNIQUE (tenant_id, submission_id, object_ref_id),
    UNIQUE (tenant_id, object_store, object_key)
);
CREATE INDEX IF NOT EXISTS idx_trace_object_refs_submission ON trace_object_refs(tenant_id, submission_id, artifact_kind);

CREATE TABLE IF NOT EXISTS trace_derived_records (
    derived_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('current', 'invalidated', 'superseded', 'revoked', 'expired')),
    worker_kind TEXT NOT NULL CHECK (worker_kind IN ('server_rescrub', 'summary', 'duplicate_precheck', 'embedding', 'ranking', 'benchmark_conversion', 'process_evaluation')),
    worker_version TEXT NOT NULL,
    input_object_ref_id TEXT,
    input_hash TEXT NOT NULL,
    output_object_ref_id TEXT,
    canonical_summary TEXT,
    canonical_summary_hash TEXT,
    task_success TEXT,
    privacy_risk TEXT CHECK (privacy_risk IS NULL OR privacy_risk IN ('low', 'medium', 'high')),
    event_count INTEGER,
    tool_sequence TEXT NOT NULL DEFAULT '[]',
    tool_categories TEXT NOT NULL DEFAULT '[]',
    coverage_tags TEXT NOT NULL DEFAULT '[]',
    duplicate_score REAL,
    novelty_score REAL,
    cluster_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, input_object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, output_object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    UNIQUE (tenant_id, submission_id, derived_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_derived_current ON trace_derived_records(tenant_id, submission_id, status, worker_kind);
CREATE INDEX IF NOT EXISTS idx_trace_derived_summary_hash ON trace_derived_records(tenant_id, canonical_summary_hash);

CREATE TABLE IF NOT EXISTS trace_vector_entries (
    vector_entry_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    derived_id TEXT NOT NULL,
    vector_store TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    embedding_dimension INTEGER NOT NULL CHECK (embedding_dimension > 0),
    embedding_version TEXT NOT NULL,
    source_projection TEXT NOT NULL CHECK (source_projection IN ('canonical_summary', 'redacted_messages', 'redacted_tool_sequence')),
    source_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'invalidated', 'deleted')),
    nearest_trace_ids TEXT NOT NULL DEFAULT '[]',
    cluster_id TEXT,
    duplicate_score REAL,
    novelty_score REAL,
    indexed_at TEXT,
    invalidated_at TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id) ON DELETE CASCADE,
    UNIQUE (tenant_id, submission_id, vector_entry_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_vector_entries_source ON trace_vector_entries(tenant_id, submission_id, status);

CREATE TABLE IF NOT EXISTS trace_audit_events (
    audit_event_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    job_id TEXT,
    submission_id TEXT,
    object_ref_id TEXT,
    export_manifest_id TEXT,
    action TEXT NOT NULL CHECK (action IN ('submit', 'read', 'review', 'credit_mutate', 'revoke', 'export', 'retain', 'purge', 'vector_index', 'benchmark_convert', 'process_evaluate')),
    reason TEXT,
    request_id TEXT,
    decision_inputs_hash TEXT,
    metadata_kind TEXT NOT NULL DEFAULT 'empty' CHECK (metadata_kind IN ('empty', 'submission', 'review_decision', 'export', 'maintenance')),
    metadata TEXT NOT NULL DEFAULT '{}',
    previous_event_hash TEXT,
    event_hash TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_audit_events_target ON trace_audit_events(tenant_id, submission_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_audit_events_action ON trace_audit_events(tenant_id, action, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_credit_ledger (
    credit_event_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    credit_account_ref TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('accepted', 'privacy_rejection', 'duplicate_rejection', 'benchmark_conversion', 'regression_catch', 'training_utility', 'reviewer_bonus', 'abuse_penalty')),
    points_delta TEXT NOT NULL,
    reason TEXT NOT NULL,
    external_ref TEXT,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    settlement_state TEXT NOT NULL CHECK (settlement_state IN ('pending', 'final', 'reversed')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_trace_credit_ledger_account ON trace_credit_ledger(tenant_id, credit_account_ref, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_credit_ledger_submission ON trace_credit_ledger(tenant_id, submission_id);

CREATE TABLE IF NOT EXISTS trace_export_manifests (
    export_manifest_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    requested_by_principal_ref TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('replay_dataset', 'benchmark_eval', 'ranking_training', 'model_training', 'analytics')),
    consent_scope_filter TEXT NOT NULL DEFAULT '[]',
    allowed_use_filter TEXT NOT NULL DEFAULT '[]',
    review_state_filter TEXT NOT NULL DEFAULT '[]',
    privacy_risk_filter TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL CHECK (status IN ('planned', 'running', 'complete', 'failed', 'revoked_invalid', 'expired_invalid')),
    item_count INTEGER NOT NULL DEFAULT 0,
    manifest_object_ref_id TEXT,
    result_object_ref_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    invalidated_at TEXT,
    FOREIGN KEY (tenant_id, manifest_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id),
    FOREIGN KEY (tenant_id, result_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id),
    UNIQUE (tenant_id, export_manifest_id)
);

CREATE TABLE IF NOT EXISTS trace_export_manifest_items (
    export_manifest_id TEXT NOT NULL REFERENCES trace_export_manifests(export_manifest_id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    derived_id TEXT,
    object_ref_id TEXT,
    vector_entry_id TEXT,
    source_status_at_export TEXT NOT NULL,
    source_hash_at_export TEXT NOT NULL,
    revoked_after_export_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (export_manifest_id, tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, vector_entry_id) REFERENCES trace_vector_entries(tenant_id, submission_id, vector_entry_id)
);

CREATE TABLE IF NOT EXISTS trace_benchmark_artifacts (
    benchmark_artifact_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    derived_id TEXT,
    benchmark_kind TEXT NOT NULL CHECK (benchmark_kind IN ('replay', 'process_eval', 'regression_case', 'ranking_pair')),
    artifact_version TEXT NOT NULL,
    object_ref_id TEXT NOT NULL,
    requirements_hash TEXT,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'approved', 'published', 'invalidated', 'deleted')),
    created_by_job_id TEXT,
    published_export_manifest_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    invalidated_at TEXT,
    deleted_at TEXT,
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, published_export_manifest_id) REFERENCES trace_export_manifests(tenant_id, export_manifest_id)
);

CREATE TABLE IF NOT EXISTS trace_tombstones (
    tombstone_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id TEXT NOT NULL,
    trace_id TEXT,
    redaction_hash TEXT,
    canonical_summary_hash TEXT,
    reason TEXT NOT NULL,
    first_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    effective_at TEXT NOT NULL,
    retain_until TEXT,
    created_by_principal_ref TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (tenant_id, submission_id)
);
CREATE INDEX IF NOT EXISTS idx_trace_tombstones_hashes ON trace_tombstones(tenant_id, redaction_hash, canonical_summary_hash);

CREATE TABLE IF NOT EXISTS trace_retention_jobs (
    retention_job_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    cutoff_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'dry_run', 'running', 'complete', 'failed', 'paused')),
    selected_count INTEGER NOT NULL DEFAULT 0,
    purged_count INTEGER NOT NULL DEFAULT 0,
    failed_count INTEGER NOT NULL DEFAULT 0,
    dry_run_report_object_ref_id TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (tenant_id, dry_run_report_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id)
);

CREATE TABLE IF NOT EXISTS trace_retention_job_items (
    retention_job_id TEXT NOT NULL REFERENCES trace_retention_jobs(retention_job_id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('expire', 'revoke', 'delete_object', 'delete_vector', 'invalidate_export', 'write_tombstone')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'done', 'failed', 'skipped_policy_changed')),
    object_ref_id TEXT,
    vector_entry_id TEXT,
    export_manifest_id TEXT REFERENCES trace_export_manifests(export_manifest_id),
    verified_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (retention_job_id, tenant_id, submission_id, action),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, vector_entry_id) REFERENCES trace_vector_entries(tenant_id, submission_id, vector_entry_id)
);
```

### Rust Store Contract Shape

The initial Rust contract now lives in `src/trace_corpus_storage.rs`. `TraceCorpusStore` is part of the shared `Database` trait because both PostgreSQL and libSQL implementations exist in this branch. `src/bin/trace_commons_ingest.rs` still serves file-backed responses, but it can mirror submit/review/credit/revoke mutations into the configured DB for dark-launch verification.

The first implementation-facing shape should stay close to:

```rust
#[async_trait]
pub trait TraceCorpusStore: Send + Sync {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError>;

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError>;

    async fn upsert_trace_tenant_policy(
        &self,
        policy: TraceTenantPolicyWrite,
    ) -> Result<TraceTenantPolicyRecord, DatabaseError>;

    async fn get_trace_tenant_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<TraceTenantPolicyRecord>, DatabaseError>;

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError>;

    async fn append_trace_object_ref(&self, object_ref: TraceObjectRefWrite) -> Result<(), DatabaseError>;
    async fn append_trace_derived_record(&self, derived_record: TraceDerivedRecordWrite) -> Result<(), DatabaseError>;
    async fn append_trace_audit_event(&self, audit_event: TraceAuditEventWrite) -> Result<(), DatabaseError>;
    async fn append_trace_credit_event(&self, credit_event: TraceCreditEventWrite) -> Result<(), DatabaseError>;
    async fn write_trace_tombstone(&self, tombstone: TraceTombstoneWrite) -> Result<(), DatabaseError>;
}
```

The concrete backend methods must take `tenant_id` as an explicit argument or as part of every write struct. Avoid generic `get_by_id` helpers for tenant-scoped trace rows.

The names below are intentionally close to the MVP concepts, but are not proposed migrations yet. All primary records carry `tenant_id`, `created_at`, and `updated_at` unless noted. All tenant-scoped reads must filter by `tenant_id` through row policy or an equivalent query guard.

### Tenants and Access Grants

`trace_tenants`

| Column | Purpose |
|--------|---------|
| `tenant_id` | Auth-derived tenant id. |
| `display_name` | Operator-facing label. |
| `status` | `active`, `suspended`, `retention_only`, `deleted`. |
| `data_residency_region` | Region pin for DB/object/vector placement. |
| `default_retention_policy_id` | Fallback central retention policy. |

`trace_tenant_policies`

| Column | Purpose |
|--------|---------|
| `tenant_id` | Tenant boundary and primary key. |
| `policy_version` | Operator-defined policy version applied during ingest. |
| `allowed_consent_scopes` | Consent scopes accepted for new submissions. |
| `allowed_uses` | Trace-card uses accepted for new submissions. |
| `updated_by_principal_ref` | Admin or job principal that last changed policy. |

`trace_access_grants`

| Column | Purpose |
|--------|---------|
| `grant_id` | Stable grant id. |
| `tenant_id` | Tenant boundary. |
| `principal_ref` | Hash or external subject id from AuthN. |
| `role` | `contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`, `vector_worker`, `benchmark_worker`, `process_eval_worker`. |
| `allowed_scopes` | Consent scopes or ABAC scope list. |
| `allowed_uses` | Debugging, evaluation, benchmark, ranking, training, analytics. |
| `expires_at` | Optional expiry for short-lived grants. |
| `revoked_at` | Revocation timestamp. |
| `created_by`, `revoked_by`, `reason` | Audit-friendly provenance. |

Access grants authorize service operations. Envelope contributor fields remain attribution only.
In the current ingest service, `export_worker` is limited to replay/ranker export surfaces,
`benchmark_worker` is limited to benchmark conversion, `retention_worker` is limited to
retention/cache cleanup maintenance, and `vector_worker` is limited to vector-index
maintenance. `process_eval_worker` is limited to writing bounded process-evaluation
metadata for accepted submissions and, when supplied with an external reference, appending
idempotent `training_utility` delayed credit for the evaluated accepted submission. These
worker roles are intentionally not treated as reviewers for generic trace listing, audit
reads, policy administration, review decisions, or unrestricted credit mutation.

### Submissions

`trace_submissions`

| Column | Purpose |
|--------|---------|
| `submission_id` | Client-provided UUID, unique within tenant. |
| `tenant_id` | Auth-derived tenant. |
| `trace_id` | Envelope trace id. |
| `auth_principal_ref` | Authenticated principal that submitted. |
| `contributor_pseudonym` | Envelope pseudonymous contributor id, nullable. |
| `submitted_tenant_scope_ref` | Envelope tenant scope ref for analytics only. |
| `schema_version` | Envelope schema version. |
| `consent_policy_version` | Submitted consent policy version. |
| `consent_scopes` | Normalized list or join table. |
| `allowed_uses` | Centralized allowed-use projection. |
| `retention_policy_id` | Central retention policy selected at ingest. |
| `status` | `received`, `accepted`, `quarantined`, `rejected`, `revoked`, `expired`, `purged`. |
| `privacy_risk` | Server-computed residual risk. |
| `redaction_pipeline_version` | Server accepted/re-scrubbed pipeline. |
| `redaction_counts` | Safe aggregate redaction label counts; never raw Privacy Filter spans. |
| `redaction_hash` | Hash over redacted content projection. |
| `canonical_summary_hash` | Duplicate precheck key. |
| `submission_score` | Current scoring result. |
| `credit_points_pending` | Mutable pending credit estimate; do not count it as settled credit. |
| `credit_points_final` | Explicit final credit snapshot when settled. Missing values are treated as `0` for aggregate settled totals. |
| `received_at`, `reviewed_at`, `revoked_at`, `expires_at`, `purged_at` | Lifecycle timestamps. |

Indexes: `(tenant_id, submission_id) unique`, `(tenant_id, status, received_at)`, `(tenant_id, canonical_summary_hash)`, `(tenant_id, expires_at)`, `(tenant_id, contributor_pseudonym)`.

### Envelopes and Object References

`trace_object_refs`

| Column | Purpose |
|--------|---------|
| `object_ref_id` | Stable object reference id. |
| `tenant_id` | Tenant boundary. |
| `submission_id` | Source submission. |
| `artifact_kind` | `submitted_envelope`, `rescrubbed_envelope`, `review_snapshot`, `benchmark_artifact`, `export_artifact`, `worker_intermediate`. |
| `object_store` | Bucket/provider alias. |
| `object_key` | Opaque key, preferably content-addressed under tenant partition. |
| `content_sha256` | Integrity hash over ciphertext or canonical plaintext projection. |
| `encryption_key_ref` | KMS key or envelope-encryption key reference. |
| `size_bytes` | Object size. |
| `compression` | Optional compression. |
| `created_by_job_id` | Producing worker/job. |
| `valid_from`, `invalidated_at`, `deleted_at` | Artifact lifecycle. |

Object keys must not reveal raw user ids, local paths, prompt content, or secrets. The service, not reviewer/user tokens, owns object-store credentials. Any backend write that stores an object reference must prove the referenced object belongs to the same `(tenant_id, submission_id)` as the row being written; bare UUID lookups are not sufficient in a multitenant corpus.

### Derived Records

`trace_derived_records`

| Column | Purpose |
|--------|---------|
| `derived_id` | Stable derived record id. |
| `tenant_id`, `submission_id`, `trace_id` | Source linkage. |
| `status` | Mirrors source eligibility: `current`, `invalidated`, `superseded`, `revoked`, `expired`. |
| `worker_kind` | `server_rescrub`, `summary`, `duplicate_precheck`, `embedding`, `ranking`, `benchmark_conversion`, `process_evaluation`. |
| `worker_version` | Version of producing code/model/policy. |
| `input_object_ref_id` | Exact input artifact, resolved only through `(tenant_id, submission_id, object_ref_id)`. |
| `input_hash` | Hash of input projection. |
| `output_object_ref_id` | Optional large output, resolved only through `(tenant_id, submission_id, object_ref_id)`. |
| `canonical_summary` | Redacted short summary when safe for DB. |
| `canonical_summary_hash` | Hash for duplicate checks. |
| `summary_model` | Producing summarizer/model or deterministic summary policy id. |
| `task_success`, `privacy_risk`, `event_count` | Queryable attributes. |
| `tool_sequence`, `tool_categories`, `coverage_tags` | Queryable arrays or join tables. |
| `duplicate_score`, `novelty_score`, `cluster_id` | Utility metadata. |

Process-evaluation derived rows should use `worker_kind = process_evaluation`, store the
evaluator version in `worker_version`, keep label and rubric output in bounded metadata or
an optional `worker_intermediate` object ref, and expose only safe aggregate values such as
tool-selection, argument-quality, ordering, verification, side-effect-safety ratings, and
overall score. Derived rows are versioned rather than overwritten. Process-evaluation
requests may also include a bounded utility credit delta plus external reference; the
service appends `training_utility` delayed credit idempotently and reports appended/skipped
counts without making the worker a generic credit mutator. Consumers should require
`status = current` and a non-revoked source submission.

### Vector Index Metadata

`trace_vector_entries`

| Column | Purpose |
|--------|---------|
| `vector_entry_id` | Stable id shared with vector DB. |
| `tenant_id`, `submission_id`, `derived_id` | Source linkage. |
| `vector_store` | Backend alias: pgvector, libSQL vector index, Qdrant, etc. |
| `embedding_model`, `embedding_dimension`, `embedding_version` | Rebuild metadata. |
| `source_projection` | `canonical_summary`, `redacted_messages`, `redacted_tool_sequence`. |
| `source_hash` | Hash of embedded redacted projection. |
| `status` | `active`, `invalidated`, `deleted`. |
| `nearest_trace_ids`, `cluster_id`, `duplicate_score`, `novelty_score` | Latest analysis snapshot. |
| `indexed_at`, `invalidated_at`, `deleted_at` | Lifecycle timestamps. |

Production may keep the vector payload in an external vector DB. Relational metadata remains the source of truth for revocation and rebuild.

### Audit Events

`trace_audit_events`

| Column | Purpose |
|--------|---------|
| `audit_event_id` | Append-only event id. |
| `tenant_id` | Tenant boundary. |
| `actor_principal_ref` | Human, token, or worker principal. |
| `actor_role` | Role at time of action. |
| `job_id` | Optional worker job. |
| `submission_id`, `object_ref_id`, `export_manifest_id` | Optional targets. |
| `action` | Submit, read, review, credit mutate, revoke, export, retain, purge, vector index, benchmark convert, policy update. |
| `reason` | Required for privileged actions. |
| `request_id` | API request or worker trace id. |
| `decision_inputs_hash` | Hash of reviewed policy/input projection. |
| `metadata_kind`, `metadata` | Typed `TraceAuditSafeMetadata` projection only. Backends must reject arbitrary request bodies, tool payloads, raw paths, token values, and unallowlisted JSON keys. |
| `created_at` | Append timestamp. |
| `previous_event_hash`, `event_hash` | Optional tamper-evident chain per tenant. |

Audit rows should be append-only. Corrections are new events.

### Credit Ledger

`trace_credit_ledger`

| Column | Purpose |
|--------|---------|
| `credit_event_id` | Append-only event id. |
| `tenant_id`, `submission_id`, `trace_id` | Source linkage. |
| `credit_account_ref` | Pseudonymous credit account. |
| `event_type` | Accepted, privacy rejection, duplicate rejection, benchmark conversion, regression catch, training utility, reviewer bonus, abuse penalty. |
| `points_delta` | Signed decimal. |
| `reason` | Human-readable explanation. |
| `external_ref` | Review decision, benchmark artifact, training run, or export manifest. |
| `actor_principal_ref`, `actor_role` | Mutating actor. |
| `settlement_state` | `pending`, `final`, `reversed`. |
| `created_at` | Append timestamp. |

Do not mutate historical ledger rows. Materialized credit totals can be cached separately and rebuilt.

### Export Manifests

`trace_export_manifests`

| Column | Purpose |
|--------|---------|
| `export_manifest_id` | Export job id. |
| `tenant_id` | Tenant boundary. |
| `requested_by_principal_ref` | Requesting actor. |
| `purpose` | `replay_dataset`, `benchmark_eval`, `ranking_training`, `model_training`, `analytics`. |
| `consent_scope_filter`, `allowed_use_filter`, `review_state_filter`, `privacy_risk_filter` | Export policy inputs. |
| `status` | `planned`, `running`, `complete`, `failed`, `revoked_invalid`, `expired_invalid`. |
| `item_count` | Source trace count. |
| `manifest_object_ref_id` | Object ref for full manifest if large. |
| `result_object_ref_id` | Object ref for export payload. |
| `created_at`, `completed_at`, `invalidated_at` | Lifecycle timestamps. |

`trace_export_manifest_items`

| Column | Purpose |
|--------|---------|
| `export_manifest_id`, `tenant_id`, `submission_id` | Source trace membership. |
| `derived_id`, `object_ref_id`, `vector_entry_id` | Exact artifact versions used. |
| `source_status_at_export`, `source_hash_at_export` | Verification snapshot. |
| `source_invalidated_at`, `source_invalidation_reason` | Set when revocation, expiration, or purge invalidates a prior export item. |

Every export item needs an audit event or an audit batch event with a cryptographic item list hash. The pilot replay, benchmark, and ranker export paths already write a deterministic source-list hash into both the exported artifact/manifest and the mirrored audit `decision_inputs_hash`; benchmark and ranker exports also persist file-backed provenance manifests, and replay dataset exports promote that hash plus item-level source snapshots into durable DB manifests.

Pilot `V29` implements the compact `trace_export_manifests` control row for replay dataset exports in both PostgreSQL and libSQL. It stores tenant id, export manifest id, artifact kind, purpose, audit event id, source submission ids, source-list hash, item count, generation time, and invalidation/deletion timestamps. Pilot `V30` adds `trace_export_manifest_items` rows for each replay export source, including source object ref ids, source status/hash snapshots, and per-item revocation, expiration, or purge invalidation.

### Benchmark Artifacts

`trace_benchmark_artifacts`

| Column | Purpose |
|--------|---------|
| `benchmark_artifact_id` | Stable artifact id. |
| `tenant_id`, `submission_id`, `derived_id` | Source linkage. |
| `benchmark_kind` | `replay`, `process_eval`, `regression_case`, `ranking_pair`. |
| `artifact_version` | Conversion schema version. |
| `object_ref_id` | Encrypted object payload. |
| `requirements_hash` | Required tools/assertions/environment. |
| `status` | `candidate`, `approved`, `published`, `invalidated`, `deleted`. |
| `created_by_job_id` | Conversion worker. |
| `published_export_manifest_id` | Optional export linkage. |
| `created_at`, `invalidated_at`, `deleted_at` | Lifecycle timestamps. |

Benchmark conversion must fail closed if the source is revoked, expired, not approved for the target use, missing replay metadata, or out of policy.

### Tombstones and Retention Jobs

`trace_tombstones`

| Column | Purpose |
|--------|---------|
| `tombstone_id` | Stable id. |
| `tenant_id`, `submission_id` | Revoked/purged submission. |
| `trace_id` | Trace id if retained by policy. |
| `redaction_hash`, `canonical_summary_hash` | Re-ingest/export prevention keys. |
| `reason` | Contributor revocation, policy expiry, admin purge, abuse. |
| `first_seen_at`, `effective_at` | Idempotency timestamps. |
| `retain_until` | Tombstone retention window. |
| `created_by_principal_ref` | Actor or worker. |

Tombstones should outlive content deletion long enough to prevent re-ingest or re-export of the same material.

`trace_retention_jobs`

| Column | Purpose |
|--------|---------|
| `retention_job_id` | Job id. |
| `tenant_id` | Tenant boundary or system tenant for cross-tenant scheduler metadata. |
| `policy_id` | Central retention policy. |
| `cutoff_at` | Selection cutoff. |
| `status` | `planned`, `dry_run`, `running`, `complete`, `failed`, `paused`. |
| `selected_count`, `purged_count`, `failed_count` | Job counters. |
| `dry_run_report_object_ref_id` | Safety review artifact. |
| `started_at`, `completed_at` | Lifecycle timestamps. |

`trace_retention_job_items`

| Column | Purpose |
|--------|---------|
| `retention_job_id`, `tenant_id`, `submission_id` | Selected source. |
| `action` | `expire`, `revoke`, `delete_object`, `delete_vector`, `invalidate_export`, `write_tombstone`. |
| `status` | `pending`, `done`, `failed`, `skipped_policy_changed`. |
| `object_ref_id`, `vector_entry_id`, `export_manifest_id` | Optional target. |
| `verified_at` | Post-action verification time. |

Retention jobs must be resumable and must verify that tenant, policy, consent, and revocation state still match immediately before destructive actions.

## PostgreSQL and libSQL Parity Notes

PostgreSQL should be the production control plane for multi-tenant service deployments. Use native UUIDs, enums or checked text, JSONB for small metadata projections, GIN indexes for tags/scopes, row-level security for tenant isolation, and transactional migration support. If vectors stay in Postgres, use pgvector for approved redacted embeddings; otherwise keep vector metadata in Postgres and use an external vector store for payload/search.

libSQL remains useful for local development, tests, and small deployments. It can mirror the same logical schema with text UUIDs, checked text enums, JSON text, integer booleans, FTS5 where needed, and `libsql_vector_idx` only for local vector workflows. Because libSQL does not provide PostgreSQL-style row-level security, tenant isolation must be enforced by repository methods, query builders, and integration tests that prove every call site supplies `tenant_id`.

Parity rules:

- Keep table names, logical columns, status values, and state transitions identical.
- Store timestamps as timestamptz in PostgreSQL and RFC3339 UTC text in libSQL.
- Use numeric/decimal for credit in PostgreSQL; use integer minor units or text decimal in libSQL to avoid float drift in ledgers.
- Treat arrays as join tables when query correctness matters across both backends. JSON arrays are acceptable only for non-authoritative display metadata.
- Put redacted envelope bodies and large artifacts in object storage for both backends. libSQL should not become the object store except in tests.
- Keep object refs and vector metadata in DB; keep object payloads in encrypted object storage; keep vector payloads in vector DB or backend-specific vector index.
- Implement DB trait operations at the shared trait first when this becomes code, then add PostgreSQL and libSQL implementations together.

## Rollout Plan

1. Define storage contracts without changing ingest behavior.
   - Freeze status enums, object ref kinds, credit event types, audit actions, retention actions, and export purposes.
   - Add serialization fixtures that map current file-backed records to the proposed logical rows.

2. Add relational metadata behind a dark-launch flag.
   - Keep file storage as the served path.
   - Dual-write submission metadata, derived metadata, credit events, tombstones, and audit events to DB.
   - Store envelope payloads in the existing file store and write DB object refs pointing at those files during the bridge phase.

3. Add encrypted object storage.
   - On new ingest, write server re-scrubbed envelopes to object storage and record `trace_object_refs`.
   - Keep file-backed reads compatible by falling back from DB object ref to existing path layout; require active object refs with `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true` for production review decisions, `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true` for production replay export reads, and `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true` for production benchmark/ranker source validation. Use `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true` to skip plaintext submitted/reviewed envelope body files once DB/object-ref review guards and the service-local encrypted object store are configured. Use `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true` to skip plaintext benchmark/ranker derived export files once DB reviewer reads, export guardrails, required source refs, and service-local object storage are configured.
   - Verify object integrity by hash before review/export reads.

4. Add vector worker as a derived-artifact consumer.
   - Index only accepted, unrevoked, unexpired, approved redacted projections.
   - Write `trace_vector_entries` and `trace_derived_records` after checking revocation immediately before publish.
   - Keep novelty/duplicate scores advisory until reconciliation jobs are green.

5. Switch reads to DB-first. This remains the next major storage cutover.
   - Contributor status, credit, review queues, analytics, and exports read from DB metadata.
   - Envelope body reads resolve through `trace_object_refs`.
   - File-backed records remain a compatibility fallback for pilot data.

6. Backfill existing file-backed data.
   - Scan each tenant directory, validate JSON, recompute hashes, create object refs, insert metadata, and append audit import events.
   - Quarantine records with validation mismatches rather than accepting silently.
   - Produce a per-tenant migration manifest with source file hashes and resulting row/object ids.

7. Enable production retention and revocation propagation.
   - Revocation writes tombstones first, then invalidates submissions, derived rows, vectors, benchmarks, and exports.
   - Retention jobs run in dry-run mode first and require verification reports before destructive deletes.
   - Destructive object/vector deletion is delayed behind a grace period.

8. Disable file-backed writes for production tenants.
   - Leave read compatibility for one release window.
   - Keep rollback by continuing DB/object dual-write until verification succeeds for all active tenants.

## Migration Verification

Each migration batch should verify:

- Every accepted/quarantined/rejected/revoked file record has exactly one `trace_submissions` row.
- Every submission has at least one envelope object ref or an explicit tombstone/purged state.
- DB `redaction_hash`, `canonical_summary_hash`, consent scopes, status, credit snapshot, and privacy risk match recomputed values.
- Credit ledger totals match local credit responses for each contributor principal.
- Sampled contributor credit/status/events, reviewer queues/lists, analytics summaries, audit event counts, and replay/export manifest listings match file-backed reader projections before a DB read flag is promoted.
- Object hashes match stored refs and decrypt under the expected tenant/key policy.
- Vector entries do not exist for revoked, rejected, quarantined, expired, or out-of-scope submissions.
- Audit import events cover every migrated submission and object ref.

## Migration and Test Checklist

Implementation checklist for the first real storage migration:

- Refresh `origin/staging` and `origin/main`; choose the next migration number after the highest migration present on either branch. Completed for `V25`.
- Add `migrations/VN__trace_corpus_storage.sql` with the PostgreSQL DDL, and update `migrations/checksums.lock` through the repo migration checksum workflow. Completed for `V25`, with vector-entry metadata added in `V28`, compact export manifest metadata added in `V29`, and export manifest item rows added in `V30`.
- Add a same-version `"trace_corpus_storage"` entry to `INCREMENTAL_MIGRATIONS` in `src/db/libsql_migrations.rs`. Completed for version `25`, with `trace_vector_entries` added in version `28`, `trace_export_manifests` added in version `29`, and `trace_export_manifest_items` added in version `30`.
- If the libSQL base `SCHEMA` is updated for fresh installs, keep the incremental migration idempotent and make sure fresh and upgraded databases converge to the same schema.
- Add `TraceCorpusStore` to the `Database` trait only after both `PgBackend` and `LibSqlBackend` implementations exist. Completed.
- Keep DB writes behind a dark-launch or dual-write flag until parity checks pass. Completed with `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- After parity checks pass, promote critical writes with `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` so DB mirror failures fail closed instead of creating file-only accepted submissions, credit events, export provenance, or audit/content-read rows.
- Keep DB reads behind surface-specific rollout flags until parity checks pass. Contributor credit/status reads are gated by `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true`, reviewer metadata reads by `TRACE_COMMONS_DB_REVIEWER_READS=true`, replay export selection by `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true`, and audit event reads by `TRACE_COMMONS_DB_AUDIT_READS=true`.
- Keep object payloads in encrypted artifact/object storage; write only object refs and hashes into DB. Completed for the local encrypted artifact sidecar, DB object-ref-backed replay envelope reads, schema-versioned benchmark conversion artifacts with audited registry/evaluation lifecycle updates, source object-ref gating for benchmark/ranker derived exports, object-primary submit/review envelope body storage, and object-primary replay export body reads; remote service-owned object storage and broader object-primary read surfaces remain future work.
- Propagate revocation and retention expiration to DB metadata before DB-first reads. Completed for submission status, tombstones, object-ref invalidation, derived-record invalidation, vector-entry invalidation, replay export manifest/item invalidation, file-backed benchmark/ranker provenance invalidation, contributor credit/status reads, reviewer metadata reads, maintenance repair of already file-marked revoked submissions, retention-expired submission/object/derived/export invalidation, and audit events for invalidation counts.
- Add a backfill tool that reads the file-backed tenant directories, validates envelopes, recomputes redaction and summary hashes, writes metadata, and emits audit import events. Initial maintenance-triggered DB mirror backfill exists for already-derived file-backed submissions and now isolates per-item failures with bounded reporting; full recompute/import manifests remain future work.
- Add a reconciliation command that compares file-backed responses with DB-backed metadata for status, review queues, credit, analytics, replay export, object refs, and tombstones. Maintenance reconciliation now covers metadata counts, invalid-source derived/export diagnostics, reader-projection parity for contributor, reviewer metadata, analytics, audit, and replay/export manifest surfaces, and compact `blocking_gaps`; `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` can require reconciliation and make remaining gaps fail closed during production promotion. Remaining work is PostgreSQL breadth, remote object storage, and broader object-primary reads.

Test checklist for the same branch:

- PostgreSQL migration test applies all migrations on an empty database and on a pre-Trace-Commons database.
- libSQL migration test applies `SCHEMA` plus `INCREMENTAL_MIGRATIONS` on an empty in-memory database and on a database initialized before the Trace Commons migration.
- Backend parity tests insert the same logical submission, object ref, audit event, credit event, derived record, vector entry, export manifest, export manifest item, and tombstone through the shared store trait for both backends. libSQL coverage exists; PostgreSQL integration coverage still needs an available test database.
- Tenant-isolation tests seed duplicate `submission_id`, `trace_id`, `canonical_summary_hash`, and contributor pseudonym under two tenants and prove all public store methods filter by tenant. Implemented for the libSQL store contract and ingest DB mirror path.
- Handler-level tests drive the future ingest/review/revoke/export callers, not only helper predicates, and assert each mocked DB/object/vector call receives `tenant_id`, `actor_principal_ref`, and `submission_id`. Implemented for submit/review/credit/revoke dual-write, DB-backed export selection, and maintenance-triggered vector metadata indexing.
- Revocation and retention propagation tests prove tombstone-first ordering and invalidation of submissions, derived rows, vectors, benchmark artifacts, exports, and credit settlement. Current coverage verifies DB tombstone/status plus object-ref, derived-row, vector-entry, and replay export manifest/item invalidation for newly discovered and already file-marked revocations, plus retention-expired DB submission/object/derived/export invalidation; benchmark settlement tests remain future work.
- Retention tests run dry-run, policy-change, legal-hold, retry, and resumed-job paths before any destructive object/vector deletion path is enabled.
- Export tests prove revoked, quarantined, rejected, expired, and out-of-scope submissions cannot enter new manifests, and existing manifests are invalidated after source revocation.
- Security tests verify PostgreSQL RLS with `app.tenant_id` and libSQL query scoping with same ids across tenants.
- Migration rollback tests prove DB-first reads can be disabled without deleting rows and that audit/tombstone rows remain append-only.

## Rollback

Rollback should be operational, not destructive:

- Keep file-backed serving path available until DB-first reads have passed verification.
- During dual-write, mark DB rows with `write_source = file_bridge` or equivalent migration metadata so they can be ignored by rollback readers.
- If object storage fails, continue accepting to file-backed storage for pilot tenants and pause DB object refs with a visible audit event.
- If DB writes fail during dual-write, accept only when the tenant is still configured for MVP mode; production tenants should fail closed after the cutover gate.
- If vector indexing misbehaves, delete or invalidate vector entries and recompute novelty from relational/hash prechecks. Do not roll back submissions.
- If retention deletion misfires, stop workers, restore from object versioning where available, and keep tombstones/audit events. Never delete audit events to hide rollback.

## Retention Safety

Retention must be centrally controlled. The envelope `trace_card.retention_policy` is an input to policy selection, not an executable instruction.

Before deleting content, a retention worker must:

- Re-read the submission through tenant policy.
- Confirm consent scope, allowed use, status, legal hold, and export membership.
- Write or confirm a tombstone.
- Invalidate derived rows, vectors, benchmark artifacts, and exports.
- Emit audit events for planned and completed actions.
- Verify object/vector deletion or mark the item failed for retry.

Use dry runs, sampled manual review, object versioning, delayed hard deletes, and per-tenant kill switches. Retention jobs should be idempotent and resumable.

## Tenant Row Policy

The trusted tenant id comes from authentication. Production request handling should bind a `TenantCtx` or ingest equivalent before any store call.

PostgreSQL policy model:

- Enable row-level security on all `trace_*` tables except global policy dictionaries.
- Set a transaction-local tenant setting such as `ironclaw.trace_tenant_id` after authentication.
- Add `USING (tenant_id = current_setting('ironclaw.trace_tenant_id', true))` and matching `WITH CHECK` policies for tenant rows.
- Give service-worker roles narrow policies for only their job type.
- Keep admin cross-tenant access behind explicit system-scope methods that always emit audit events.

libSQL policy model:

- No implicit RLS. Every repository method takes `tenant_id` and includes it in predicates.
- Avoid generic "get by id" helpers for tenant-scoped tables.
- Integration tests must seed same UUIDs across two tenants and prove cross-tenant reads, writes, review decisions, revocations, exports, and credit queries cannot cross the tenant boundary.

## Test Plan

Tenant isolation tests:

- Contributor token for tenant A cannot list, status-check, revoke, review, export, or credit-sync tenant B submissions, even when it knows `submission_id`.
- Reviewer/admin token for tenant A cannot access tenant B quarantine, analytics, audit, object refs, vectors, exports, or credit ledger rows.
- Same `submission_id`, `trace_id`, `canonical_summary_hash`, and contributor pseudonym can exist in two tenants without collisions.
- DB-backed queries include tenant predicates at the caller level, not just in low-level helpers.
- PostgreSQL RLS tests run with `ironclaw.trace_tenant_id` set to tenant A and confirm tenant B rows are invisible.
- libSQL integration tests use a shared database with two tenants and assert every public repository method scopes by tenant.

Revocation propagation tests:

- Revocation is idempotent and preserves the first revocation timestamp/reason while appending later audit context.
- Revocation writes a tombstone before content invalidation.
- After revocation, status sync reports revoked, review approval fails, credit finalizes or reverses according to policy, and dataset export excludes the source.
- Vector worker checks revocation before read and before publish; a revoked source cannot create or keep an active vector entry.
- Benchmark conversion and export jobs fail closed when revocation occurs between selection and publish.
- Existing replay export manifests and their item rows are marked invalid when a source is revoked after export.
- Retention jobs skip or alter actions when revocation or legal hold state changes after dry-run selection.
- Reconciliation finds active derived artifacts, vectors, benchmark artifacts, or exports whose source is revoked and invalidates them.

Caller-level regression tests should drive the actual handlers or store facades that perform side effects, not only helper predicates. Mocks of DB/object/vector APIs must capture tenant id, actor principal, object ref id, and submission id for every call so missing propagation is visible.

## Implementation Notes

When this plan becomes code:

- Add shared DB trait methods before backend-specific implementations.
- Implement PostgreSQL and libSQL migrations together.
- Keep all new writes behind feature flags until dual-write verification exists.
- Update `FEATURE_PARITY.md` only if user-visible Trace Commons status changes.
- Update `docs/internal/trace-commons.md` if endpoint behavior, threat model, or MVP caveats change.
- Run targeted tests for changed storage, web handlers, migration tooling, and the Rust storage contract.
