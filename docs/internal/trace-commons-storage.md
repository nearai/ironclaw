# Trace Commons Production Storage Plan

This document makes the migration path from the current file-backed Trace Commons ingest MVP to production storage concrete without wiring any new runtime behavior. It assumes the existing `ironclaw.trace_contribution.v1` envelope remains the upload contract, while production ingest moves trust, authorization, retention, review, export, and credit accounting into durable services.

## Current State

The MVP ingestion service stores tenant-scoped JSON files under `TRACE_COMMONS_DATA_DIR` and derives lightweight records for review, analytics, credit, and replay export. That is appropriate for local development and controlled pilots, but production needs stronger guarantees:

- Relational metadata for tenant policy, workflow state, credit, audit, retention, and export manifests.
- Encrypted object storage for redacted envelope bodies and large derived artifacts.
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

`trace_access_grants`

| Column | Purpose |
|--------|---------|
| `grant_id` | Stable grant id. |
| `tenant_id` | Tenant boundary. |
| `principal_ref` | Hash or external subject id from AuthN. |
| `role` | `contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`, `vector_worker`, `benchmark_worker`. |
| `allowed_scopes` | Consent scopes or ABAC scope list. |
| `allowed_uses` | Debugging, evaluation, benchmark, ranking, training, analytics. |
| `expires_at` | Optional expiry for short-lived grants. |
| `revoked_at` | Revocation timestamp. |
| `created_by`, `revoked_by`, `reason` | Audit-friendly provenance. |

Access grants authorize service operations. Envelope contributor fields remain attribution only.

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
| `redaction_hash` | Hash over redacted content projection. |
| `canonical_summary_hash` | Duplicate precheck key. |
| `submission_score` | Current scoring result. |
| `credit_points_pending` | Mutable pending credit snapshot. |
| `credit_points_final` | Final credit snapshot when settled. |
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

Object keys must not reveal raw user ids, local paths, prompt content, or secrets. The service, not reviewer/user tokens, owns object-store credentials.

### Derived Records

`trace_derived_records`

| Column | Purpose |
|--------|---------|
| `derived_id` | Stable derived record id. |
| `tenant_id`, `submission_id`, `trace_id` | Source linkage. |
| `status` | Mirrors source eligibility: `current`, `invalidated`, `superseded`, `revoked`, `expired`. |
| `worker_kind` | `server_rescrub`, `summary`, `duplicate_precheck`, `embedding`, `ranking`, `benchmark_conversion`. |
| `worker_version` | Version of producing code/model/policy. |
| `input_object_ref_id` | Exact input artifact. |
| `input_hash` | Hash of input projection. |
| `output_object_ref_id` | Optional large output. |
| `canonical_summary` | Redacted short summary when safe for DB. |
| `canonical_summary_hash` | Hash for duplicate checks. |
| `task_success`, `privacy_risk`, `event_count` | Queryable attributes. |
| `tool_sequence`, `tool_categories`, `coverage_tags` | Queryable arrays or join tables. |
| `duplicate_score`, `novelty_score`, `cluster_id` | Utility metadata. |

Derived rows are versioned rather than overwritten. Consumers should require `status = current` and a non-revoked source submission.

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
| `action` | Submit, read, review, credit mutate, revoke, export, retain, purge, vector index, benchmark convert. |
| `reason` | Required for privileged actions. |
| `request_id` | API request or worker trace id. |
| `decision_inputs_hash` | Hash of reviewed policy/input projection. |
| `metadata` | Small JSON object without trace content. |
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
| `revoked_after_export_at` | Set when revocation invalidates prior export. |

Every export item needs an audit event or an audit batch event with a cryptographic item list hash.

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
   - Keep file-backed reads compatible by falling back from DB object ref to existing path layout.
   - Verify object integrity by hash before review/export reads.

4. Add vector worker as a derived-artifact consumer.
   - Index only accepted, unrevoked, unexpired, approved redacted projections.
   - Write `trace_vector_entries` and `trace_derived_records` after checking revocation immediately before publish.
   - Keep novelty/duplicate scores advisory until reconciliation jobs are green.

5. Switch reads to DB-first.
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
- Quarantine, analytics, and replay export API responses match file-backed behavior for sampled tenants.
- Object hashes match stored refs and decrypt under the expected tenant/key policy.
- Vector entries do not exist for revoked, rejected, quarantined, expired, or out-of-scope submissions.
- Audit import events cover every migrated submission and object ref.

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
- Set a transaction-local tenant setting such as `app.tenant_id` after authentication.
- Add `USING (tenant_id = current_setting('app.tenant_id'))` and matching `WITH CHECK` policies for tenant rows.
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
- PostgreSQL RLS tests run with `app.tenant_id` set to tenant A and confirm tenant B rows are invisible.
- libSQL integration tests use a shared database with two tenants and assert every public repository method scopes by tenant.

Revocation propagation tests:

- Revocation is idempotent and preserves the first revocation timestamp/reason while appending later audit context.
- Revocation writes a tombstone before content invalidation.
- After revocation, status sync reports revoked, review approval fails, credit finalizes or reverses according to policy, and dataset export excludes the source.
- Vector worker checks revocation before read and before publish; a revoked source cannot create or keep an active vector entry.
- Benchmark conversion and export jobs fail closed when revocation occurs between selection and publish.
- Existing export manifests are marked invalid or item-level invalid when a source is revoked after export.
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
- Run targeted tests for changed storage, web handlers, and migration tooling. Cargo is not required for this documentation-only scaffold.
