# Trace Commons Roadmap

This roadmap coordinates the multiphase path from the current Trace Commons MVP to a production-ready private corpus. It is a planning companion to `docs/internal/trace-commons.md` and `docs/internal/trace-commons-storage.md`; those documents remain authoritative for the envelope rules, threat model, API surface, storage schema, and migration details.

## Current Gecko-Pass Status

As of the `gecko-pass` branch, Trace Commons has moved beyond the local-only MVP into a dark-launch production-storage bridge:

- Local capture remains opt-in, local-first, and redaction-first. Raw recorded traces still must not leave the client.
- The private ingestion service still serves file-backed pilot APIs by default, but can dual-write metadata through `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- PostgreSQL and libSQL schema slices exist through the current Trace Commons storage work: core corpus rows, object refs, derived records, vector metadata, audit events, credit ledger rows, tombstones, retention/export metadata, compact replay export manifests, and replay export item snapshots.
- `TraceCorpusStore` exists behind the shared database abstraction with backend implementations and libSQL-focused parity coverage.
- Optional DB-backed read flags now cover contributor credit/status, reviewer metadata, replay export selection, and audit event reads.
- The encrypted local artifact sidecar stores submitted redacted envelopes, and DB-backed replay export resolves bodies through a shared policy/audit helper that verifies active DB object refs, tenant scope, artifact kind, and content hash for file-backed objects or encrypted artifacts.
- Local credit visibility now has a reusable report shape that separates local lifecycle state from central accepted/quarantined/rejected status, credit totals, delayed ledger deltas, and last submission/status-sync times.
- Maintenance can backfill file-backed pilot records into the DB mirror, mark/purge expired records, prune invalid export caches, index deterministic vector metadata for canonical summaries, and run file-vs-DB reconciliation with reader-projection parity diagnostics.
- Export audit paths now carry deterministic source-list hashes, and replay export manifest metadata can be listed by reviewer/admin tokens.
- Revocation, retention expiration, and maintenance-discovered file tombstones already invalidate DB-mirrored submission status, object refs, derived records, vector metadata, replay export manifests, and replay export item rows.

The main remaining gap is production ownership: file-backed serving is still the compatibility path, encrypted artifacts are still local rather than service-owned object storage, PostgreSQL RLS/central policy is not yet the active trust boundary, and vector, benchmark, ranking, retention, and audit systems are not yet complete production workers.

## Roadmap Principles

- Keep contribution opt-in and local-first. Uploads are always redacted `ironclaw.trace_contribution.v1` envelopes.
- Treat envelope contributor and tenant fields as attribution only. Authorization comes from request identity, tenant policy, and DB row scope.
- Keep the file-backed MVP available until each DB/object-primary read surface has parity evidence and rollback.
- Store metadata, object refs, hashes, indexes, ledgers, and workflow state in DB. Store trace bodies and large artifacts in encrypted object storage. Store vector payloads in a vector backend or backend-specific index, with relational metadata as the source of truth.
- Version every derived artifact by input hash, worker version, policy version, and output artifact id.
- Test through callers for any side effect: handlers, store facades, maintenance jobs, and export/revoke flows must prove tenant id, actor principal, object ref, and submission id propagation.

## Phase Plan

### Phase 0: MVP and Storage Bridge Baseline

Status: mostly complete on `gecko-pass`.

Scope:

- Local opt-in policy, preview, queue, flush, credit display, list-submission summary visibility, scoped web/runtime policy, and autonomous post-turn contribution.
- Deterministic local redaction, tool-aware payload redaction, stable placeholders, and Privacy Filter safe projection.
- Internal ingestion service with submit, list, revoke, review, credit, analytics, replay export, benchmark candidate, maintenance, and audit surfaces.
- DB dual-write metadata, encrypted local artifact sidecar, DB-backed reader flags, vector metadata indexing, compact replay manifest rows, and maintenance backfill/reconciliation.

Dependencies:

- Existing local redaction envelope contract.
- File-backed pilot store under `TRACE_COMMONS_DATA_DIR`.
- Current PostgreSQL/libSQL migrations and shared `TraceCorpusStore`.

Verification gates:

- Local redaction canary tests prove secrets, paths, raw sidecar spans, and raw text do not survive accepted envelopes or derived summaries.
- libSQL store and caller-level tests prove tenant scoping for submit, review, credit, revoke, export selection, maintenance vector indexing, and audit reads.
- File-backed APIs remain default unless a surface-specific DB read flag is enabled.

Exit criteria:

- Keep this phase shippable as the internal pilot baseline while later phases build behind flags.

### Phase 1: DB Read Cutover Readiness

Status: in progress. Reconciliation now covers submissions, derived records,
object refs, vectors, credit-ledger counts, audit-event counts, replay/export
manifest counts, export item counts, revocation/tombstone counts, and
reader-projection parity for contributor credit, reviewer metadata, analytics,
audit, and replay/export manifest surfaces; remaining cutover work is parity
enforcement, PostgreSQL coverage breadth, and rollout diagnostics.

Scope:

- Use reconciliation parity diagnostics as the promotion gate for contributor, reviewer metadata, analytics, audit, and replay/export manifest DB read flags.
- Add PostgreSQL integration coverage for the same logical store operations already covered in libSQL.
- Make DB-backed reader flags safe to enable per tenant or per deployment surface, with visible diagnostics when parity checks fail.
- Keep file-backed fallback for pilot data and rollback during the cutover window.

Dependencies:

- Phase 0 dual-write/backfill metadata.
- Stable status values, audit actions, credit event kinds, export purposes, object ref kinds, and retention/revocation transitions.

Verification gates:

- For sampled tenants, DB-backed contributor status/credit, reviewer lists, analytics, replay export selection, and audit event reads match file-backed behavior.
- Backfill rejects or quarantines malformed pilot records instead of silently accepting them.
- PostgreSQL and libSQL both prove duplicate ids/hashes under separate tenants do not cross-read or cross-mutate.

Exit criteria:

- DB metadata reads can be promoted surface by surface without changing the envelope contract or losing file-backed rollback.

### Phase 2: Service-Owned Object Storage

Status: not production-complete.

Scope:

- Move redacted envelope bodies and large artifacts from local encrypted sidecar semantics to service-owned encrypted object storage.
- Resolve review/export/body reads through `trace_object_refs` first, with file-backed fallback only for migrated pilot records.
- Verify content hashes and tenant/key policy before every trace-content read.
- Add object lifecycle states for invalidated, deleted, and retained artifacts, and connect them to revocation/retention jobs.

Dependencies:

- Phase 1 DB metadata reads for object refs and source eligibility.
- Secrets/KMS or equivalent key reference strategy.
- Tenant-derived object partitioning that never trusts envelope tenant fields.

Verification gates:

- Object keys do not expose raw user ids, local paths, prompts, tenant tokens, or secret values.
- Object reads fail closed when DB source status is revoked, expired, purged, rejected, quarantined, or outside the requested consent/use scope.
- Hash/decrypt checks run before reviewer, replay, benchmark, ranker, or audit-visible content access.

Exit criteria:

- New production tenants can store redacted trace bodies outside file-backed pilot directories while keeping pilot fallback available for migration.

### Phase 3: Tenant Policy, RBAC, ABAC, and Audit Hardening

Status: partially represented by static token roles plus optional ingest-time
tenant submission policies for allowed consent scopes and trace-card uses.
Production-like deployments can require explicit tenant policy entries, and
ingest can now read those policies from the TraceCorpusStore behind
`TRACE_COMMONS_DB_TENANT_POLICY_READS`; fuller RBAC/ABAC remains future work.
The PostgreSQL store now sets `ironclaw.trace_tenant_id` transaction-locally around
tenant-scoped Trace Commons operations while retaining explicit `tenant_id`
predicates. This is an incremental guardrail only: table owners, superusers, and
roles with `BYPASSRLS` can still bypass the V31 policies until production role
ownership and/or forced RLS are settled.

Scope:

- Replace static tenant-token assumptions and JSON pilot policies with central tenant context, short-lived credentials or signed upload claims, role grants, allowed scopes, allowed uses, and expiry.
- Enforce PostgreSQL RLS or equivalent query guardrails for every tenant-scoped `trace_*` table.
- Keep libSQL scoping explicit in repository methods and caller tests.
- Require reasons and typed safe metadata for privileged actions: review override, delayed credit mutation, bulk export, retention override, purge, and tombstone changes.
- Add tamper-evident or append-only audit behavior for submit, read, review, credit mutate, revoke, export, retain, purge, vector index, and benchmark conversion.

Dependencies:

- Phase 1 DB metadata reads.
- Phase 2 object access through service-owned identities.
- A settled `TenantCtx` or ingest equivalent for handlers and workers.

Verification gates:

- Contributor, reviewer, admin, and worker roles are tested against same-id cross-tenant fixtures.
- Every privileged mutation emits an audit event with tenant id, actor/job id, role, target ids, reason, and decision input hash where applicable.
- PostgreSQL RLS tests prove tenant B rows are invisible under tenant A context; libSQL tests prove equivalent predicate scoping.

Exit criteria:

- Authorization no longer depends on envelope fields or static pilot tokens, and audit coverage is sufficient for security review.

### Phase 4: Derived Artifact Workers

Status: vector metadata and benchmark candidate plumbing are partial; production workers remain future work.

Scope:

- Implement private vector duplicate/novelty workers that embed only approved redacted projections and write `trace_vector_entries` plus derived records.
- Promote benchmark conversion into controlled worker jobs that record consent scope, review state, redaction version, replayability requirements, source-list hashes, and artifact refs.
- Add ranker/model-utility jobs as offline analysis that may append delayed credit only with a downstream artifact/job reference.
- Extend item-level export manifest rows beyond replay datasets to benchmark and ranker artifacts once those exports become durable job outputs.

Dependencies:

- Phase 2 object-primary artifact reads for all trace-body surfaces.
- Phase 3 worker roles, audit, and ABAC checks.
- Revocation checks before source read and before artifact publish.

Verification gates:

- No vector entry, benchmark artifact, ranker pair, or export item can be produced for revoked, expired, quarantined, rejected, out-of-scope, or unapproved submissions.
- Worker outputs record input hash, worker version, policy version, source projection, and output object ref or vector id.
- Delayed credit events are append-only, bounded by policy, reasoned, and linked to review/export/worker artifacts.

Exit criteria:

- Derived artifacts become reproducible, tenant-scoped, revocation-aware corpus assets rather than ad hoc pilot outputs.

### Phase 5: Production Retention and Revocation Propagation

Status: partial metadata invalidation exists; full worker/object/vector/export propagation remains future work.

Scope:

- Implement resumable retention jobs with dry-run reports, legal-hold checks, policy-change handling, retries, grace periods, and verification.
- Fan out revocation and retention transitions to object refs, object payloads, vectors, benchmark artifacts, ranking/training queues, export manifests/items, credit settlement, and worker queues.
- Keep tombstones long enough to prevent re-ingest or re-export after content deletion.
- Add reconciliation that finds active derived artifacts whose source is revoked, expired, or purged and invalidates them.

Dependencies:

- Phase 2 object lifecycle controls.
- Phase 3 audit and policy enforcement.
- Phase 4 derived artifact source links.

Verification gates:

- Revocation writes or confirms tombstones before content invalidation.
- Existing exports are invalidated or item-marked when a source is revoked after export.
- Destructive object/vector deletes are delayed, audited, resumable, and verified.
- Benchmark, ranking, and credit-settlement invalidation tests cover the gaps called out in the storage plan.

Exit criteria:

- Production deployments can honor contributor revocation and retention policy across every central and derived corpus surface.

### Phase 6: Production Cutover and Tenant Rollout

Status: future.

Scope:

- Disable file-backed writes for production tenants after DB/object-primary reads pass parity and rollback windows.
- Keep file-backed reads for one release window for migrated pilot tenants.
- Add per-tenant rollout flags, dashboards, maintenance reports, and kill switches for DB reads, object reads, vector workers, benchmark exports, and retention deletion.
- Update public/internal docs and `FEATURE_PARITY.md` if user-visible Trace Commons behavior changes.

Dependencies:

- Phases 1 through 5 complete for the target tenant class.

Verification gates:

- Migration manifests prove source file hashes, DB rows, object refs, tombstones, credit totals, audit events, and export manifests converge.
- Rollback drills prove DB-first reads can be disabled without deleting rows and without losing audit/tombstone state.
- Security review clears tenant policy, object access, audit, retention, and revocation paths.

Exit criteria:

- Trace Commons can accept production tenants without relying on pilot file storage as the primary serving path.

## Parallelization Lanes

These lanes can proceed in parallel as long as their write scopes stay disjoint and they meet at the verification gates above.

| Lane | Primary ownership | Can start after | Produces | Must coordinate with |
|------|-------------------|-----------------|----------|----------------------|
| A. DB parity and read cutover | Storage/control-plane | Phase 0 | Reconciliation coverage, PostgreSQL tests, safer DB read flags | Lanes B, E, G |
| B. Ingestion/API reader promotion | Ingest service/API | Phase 0 and Lane A contracts | DB-backed contributor, reviewer, analytics, replay, audit behavior by surface | Lanes A, D, E |
| C. Object-primary artifact storage | Artifact service/storage | Object ref contract from Phase 0 | Service-owned encrypted object reads/writes and local-sidecar migration path | Lanes A, B, F |
| D. Tenant policy and audit | Auth/security | Phase 0 role semantics | Tenant context, RBAC/ABAC, RLS policy, typed audit metadata | Lanes B, C, E, F |
| E. Retention and revocation propagation | Lifecycle workers | Phase 0 invalidation semantics | Tombstone-first propagation, retention jobs, reconciliation, rollback safety | Lanes A, C, F |
| F. Derived workers and exports | Vector/benchmark/ranking | Phases 2 and 3 contracts | Vector worker, benchmark worker, ranker utility, item-level manifests | Lanes C, D, E |
| G. Verification and docs | Test/operations/docs | Always | Caller-level tests, migration reports, rollout docs, parity notes | All lanes |

## Dependency Map

- DB-backed reads depend on dual-write or backfill plus reconciliation.
- Object-primary reads depend on DB object refs and tenant/key policy.
- RLS and ABAC depend on a trusted tenant context in handlers and workers.
- Vector and benchmark workers depend on object-primary reads, worker roles, audit, and revocation checks.
- Retention and revocation production gates depend on object/vector/export artifact links.
- Broad rollout depends on parity evidence, rollback drills, and docs updates.

## Verification Gates Summary

- Redaction gate: accepted envelopes and derived summaries never contain raw trace text, raw sidecar spans, secrets, local paths, bearer tokens, or raw tool payloads outside explicit policy.
- Tenant gate: every read/write/mutation/export path is driven by auth-derived tenant and actor context, with same-id cross-tenant tests.
- Parity gate: DB-backed reader-projection diagnostics are green before a surface-specific read flag is promoted.
- Object gate: every trace body read verifies object ref tenant linkage, hash, decryptability, source status, consent scope, and allowed use.
- Audit gate: privileged mutations and content reads emit typed, tenant-scoped, append-only audit events with reason and decision input hashes where needed.
- Revocation gate: revoke and retention flows invalidate or block submissions, object refs, derived rows, vectors, benchmarks, exports, worker queues, and credit settlement.
- Rollback gate: disabling DB/object/vector read flags leaves file-backed pilot behavior available and preserves audit/tombstone history.

## Next Build Lanes

The highest-value next work is:

1. Finish DB-read parity and reconciliation so reviewer, analytics, replay, audit, and contributor surfaces can graduate from optional flags with confidence.
2. Introduce service-owned encrypted object storage and route remaining review/export body reads through object refs.
3. Add tenant policy/RLS hardening before broadening reviewer/admin/export access.
4. Complete retention/revocation propagation for benchmark, ranking, worker, and already-published export artifacts.
5. Build the private vector worker and benchmark conversion workers only after object-primary reads and worker authorization are in place.

This ordering keeps the corpus trustworthy before it becomes more useful: metadata parity and object ownership come first, then policy/audit hardening, then derived data products.
