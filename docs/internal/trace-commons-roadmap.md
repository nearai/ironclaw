# Trace Commons Roadmap

This roadmap coordinates the multiphase path from the current Trace Commons MVP to a production-ready private corpus. It is a planning companion to `docs/internal/trace-commons.md` and `docs/internal/trace-commons-storage.md`; those documents remain authoritative for the envelope rules, threat model, API surface, storage schema, and migration details.

## Production Gap Queue

These are the next independent production slices that can be staffed in parallel. Each slice should land behind flags where it changes serving behavior and must add caller-level tenant-scope tests before promotion.

### Auth and Keying

- [x] Add EdDSA/Ed25519 signed upload-claim verification through default or `kid`-selected public-key config, JSON/file keysets with optional `not_before`/`not_after` activation windows, safe config-status total/EdDSA active/inactive key counts, issuer/audience/JTI/TTL policy, unsupported-algorithm rejection, and an EdDSA-required production gate that rejects static tokens and HS256 signed claims on authenticated routes.
- [x] Add issuer-managed EdDSA/Ed25519 upload-claim enforcement that accepts only active `kid`-selected keys from the managed JSON/file keyset, requires issuer and audience checks, and rejects static tokens, HS256 claims, default EdDSA keys, ad hoc keyed public-key files, missing `kid`, and unmanaged `kid` values.
- [x] Add guarded remote issuer-managed EdDSA/Ed25519 keyset bootstrap with HTTPS-only fetch, exact host allowlist, no redirects, DNS/private-target validation, DNS pinning, optional bearer fetch credential, timeout, and body-size cap.
- [x] Add autonomous client upload-claim refresh for opted-in queues: scoped policies can point at a guarded HTTPS issuer, request short-lived EdDSA/Ed25519 bearer claims with tenant/audience/consent/use metadata, keep workload credentials in env vars, cache claims only in memory until the refresh margin, and retry submit/status-sync once after 401/403 with a forced refresh.
- [x] Finish live issuer-managed EdDSA/Ed25519 key refresh/sync so long-running deployments can rotate issuer-owned key records after startup without restart: URL keysets refresh in-process on a bounded interval, failures preserve the last good active keyset, managed-required deployments can configure a max-stale fail-closed window, and config-status exposes safe refresh health without URLs, hosts, key ids, PEMs, or bearer fetch credentials.
- [x] Add durable tenant access grant storage for issuer-authorized hosted-agent principals, roles, consent scopes, allowed uses, issuer/audience/subject attribution, expiry, revocation metadata, and safe metadata across PostgreSQL/libSQL, plus admin create/list/revoke APIs, CLI helpers, and enforcement gates for trace submission plus autonomous contributor credit/status readback that require an active exact-role grant and intersect grant scopes/uses with static or EdDSA claim allow-lists.
- [ ] Promote auth-derived `TenantCtx` into every ingest, review, export, worker, maintenance, and contributor-status path so envelope tenant fields remain attribution only.
- [ ] Harden PostgreSQL tenant isolation with production roles, transaction-local tenant context coverage, RLS/forced-RLS decisions, and same-id cross-tenant tests; keep libSQL predicate tests in parallel. Admin config-status now exposes safe PostgreSQL RLS readiness diagnostics for policy coverage, expression drift, disabled RLS, force-RLS count, and role bypass state.

### Autonomous Client

- [ ] Turn opted-in post-turn capture into a durable background worker with retry/backoff, idempotency, and network-offline handling. Current runtime capture already skips ineligible current traces instead of leaving new held files, and the agent runtime now also runs a periodic scoped queue flush worker with typed retry/backoff sidecars plus durable scoped flush/status-sync telemetry. Queue flush now compacts duplicate queued contribution envelopes and orphan held sidecars before submission while preserving local-first body isolation, can refresh short-lived EdDSA upload claims from a guarded issuer, and current telemetry splits sanitized network failures into offline, DNS, timeout, connection-refused, and generic network buckets. Remaining work is richer request-boundary classification from provider error types before stringification.
- [x] Add periodic contributor credit notices across CLI/web/runtime that summarize accepted, quarantined, rejected, revoked, pending/final credit, delayed-credit deltas, and credit-event counts without exposing trace bodies or central corpus rows. Current notices can be acknowledged until the local credit fingerprint changes or snoozed for a bounded period through CLI and authenticated web actions.
- [x] Add first local queue diagnostics/status surfaces for CLI and authenticated web: policy readiness, bearer-token environment presence, queued/held counts, sanitized held-reason counts, and local credit summaries where available.
- [x] Extend autonomous diagnostics with policy/version mismatch promotion guidance. Durable last-attempt/success/failure flush telemetry, retryable submission failure counters, status-sync counters, retry/backoff state, last compaction reclaimed count, duplicate-envelope and orphan-hold-sidecar compaction counters, aggregated schema-version/consent-policy/redaction-pipeline/trace-card-redaction-pipeline/malformed-envelope warnings, warning severity, production-promotion blocking flags, safe recommended actions, and sanitized Endpoint/Credential/Network/NetworkOffline/NetworkDns/NetworkTimeout/NetworkConnectionRefused/HttpRejection/Policy/Queue/StatusSync/Submission/Unknown failure classes are now exposed locally through scoped queue diagnostics. Warning aggregates do not include raw envelope bodies or raw observed mismatch values.

### Ingestion Storage

- [ ] Promote DB/object-primary submit, review, replay, benchmark, and ranker paths from pilot flags to per-tenant rollout flags after reconciliation parity is green. Initial tenant allowlist gates now wrap DB contributor/reviewer/replay/audit/tenant-policy reads, object-ref-required modes, and object-primary submit/replay/derived flags; contributor-credit and object-primary submit caller tests cover tenant A canary rollout while tenant B remains on file fallback.
- [ ] Replace service-local encrypted artifact storage with a service-owned object-store provider abstraction, KMS/key-ref strategy, tenant-hashed object keys, hash/decrypt verification, and migration/backfill tooling. The local encrypted store now implements a `TraceArtifactStore` provider trait for serialized JSON write/read/delete conformance; remote object/KMS providers and migration tooling remain.
- [ ] Add PostgreSQL integration coverage matching the current libSQL `TraceCorpusStore` slices for submissions, object refs, derived records, vectors, audit, credit, retention jobs, export manifests/items, policies, and tombstones. Retention job/item facade scope, tenant-policy update/scope behavior, review-lease claim/release/audit behavior, raw RLS visibility, and export-manifest mirror rollback atomicity now have PostgreSQL coverage.

### Review and Governance

- [ ] Extend review leases into a fuller assignment/escalation workflow with SLA filters, batch actions, privacy-review reasons, and central reviewer routing. Review finalization now rejects non-quarantined, terminal, expired, and aggregate-only approval records before trace content is read, while PostgreSQL facade tests cover tenant-scoped lease claim/release ownership and typed lease audit metadata.
- [ ] Complete remaining privileged-action ABAC for review override, purge, and tombstone changes using tenant policy plus signed-claim allowed scopes/uses. Manual reviewer/admin delayed utility credit now enforces source allowed-use, tenant-policy, and signed-claim ABAC like worker utility credit.
- [ ] Make audit append/read paths production-grade: DB-primary hash-chain verification, per-source content-read rows, reason enforcement, sampled reconciliation, and no broad corpus download path.

### Datasets

- [ ] Replace deterministic vector similarity with a private embedding worker that reads only approved redacted projections, writes tenant-scoped vector metadata, and invalidates entries on revocation/retention.
- [ ] Promote benchmark conversion and ranker training exports into durable worker jobs with source-list hashes, artifact object refs, lifecycle state, replayability checks, and idempotent delayed utility credit. Export call sites now mirror one-shot durable grant rows and running/complete job rows; background worker execution remains.
- [ ] Add export governance for replay, benchmark, ranker, and training slices: explicit purpose, consent/use filters, item caps, source object refs, manifest invalidation, and time-limited controlled job access. Replay, benchmark, ranker-candidate, and ranker-pair exports now validate and persist tenant/principal/purpose/dataset-kind grant/job slices.

### Observability

- [ ] Add operational dashboards or API summaries for queue throughput, accept/quarantine/reject rates, redaction risk, review SLA, export volume, retention jobs, vector coverage, and delayed credit settlement.
- [ ] Emit structured metrics/logs for every promotion gate: DB/file parity, object-ref readability/hash failures, RLS/predicate denials, signed-claim failures, worker skips, and revoked-source invalidations. PostgreSQL RLS catalog readiness is now available through safe admin config-status diagnostics; runtime metrics remain.
- [ ] Build runbooks and smoke checks for per-tenant rollout, rollback, key rotation, object-store migration, retention purge dry runs, and audit-chain verification.

## Current Gecko-Pass Status

As of the `gecko-pass` branch, Trace Commons has moved beyond the local-only MVP into a dark-launch production-storage bridge:

- Local capture remains opt-in, local-first, and redaction-first. Raw recorded traces still must not leave the client.
- Autonomous clients now have first-pass queue diagnostics/status: CLI `traces queue-status` reports scoped readiness, bearer-token environment presence, queue/hold counts, typed retry/manual-review/policy hold counts, next retry time, durable flush/status-sync telemetry, last compaction reclaimed count, duplicate-envelope and orphan-hold-sidecar removals, safe warning aggregates for schema-version/consent-policy/redaction-pipeline/trace-card-redaction-pipeline mismatches and malformed envelopes, sanitized failure classes, sanitized held-reason counts, and local credit summary fields; the authenticated web API exposes scoped queue/held counts, durable telemetry, safe warning aggregates, and sanitized held entries without exposing envelope bodies or raw observed mismatch values.
- Periodic credit notices now run through CLI, web, post-turn runtime, and the periodic queue worker path, including delayed ledger deltas and credit-event counts without surfacing trace bodies or central corpus rows. CLI and authenticated web clients can also acknowledge the current local credit fingerprint or snooze notices for a bounded number of hours.
- Signed upload-claim auth supports EdDSA/Ed25519 public-key verification through default or `kid`-selected keys and JSON/file/guarded-HTTPS keysets with optional activation windows and safe active/inactive/managed key-count diagnostics, while HS256 claims and static tokens remain internal bridge paths. A managed-EdDSA-required gate now accepts only active managed-keyset claims with issuer/audience checks; autonomous clients can fetch short-lived EdDSA upload claims from guarded tenant issuers, and ingestion services can refresh guarded remote issuer-managed Ed25519 keysets live after startup with last-good preservation and optional max-stale fail-closed enforcement.
- The private ingestion service still serves file-backed pilot APIs by default, but can dual-write metadata through `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- PostgreSQL and libSQL schema slices exist through the current Trace Commons storage work: core corpus rows, object refs, derived records, vector metadata, audit events, credit ledger rows, tombstones, retention/export metadata, compact replay export manifests, and replay export item snapshots.
- `TraceCorpusStore` exists behind the shared database abstraction with backend implementations and libSQL-focused parity coverage.
- Optional DB-backed read flags now cover contributor credit/status, reviewer metadata, replay export selection, and audit event reads.
- The encrypted local artifact sidecar stores submitted redacted envelopes, and DB-backed replay export resolves bodies through a shared policy/audit helper that verifies active DB object refs, tenant scope, artifact kind, and content hash for file-backed objects or encrypted artifacts. Production-shaped object-primary modes can now skip plaintext submit/review bodies, replay-export body fallback, and benchmark/ranker derived export files when the matching DB/object-store guards are enabled.
- Local credit visibility now has a reusable report shape that separates local lifecycle state from central accepted/quarantined/rejected status, credit totals, delayed ledger deltas, and last submission/status-sync times.
- Maintenance can backfill file-backed pilot records into the DB mirror, mark/purge expired records, prune invalid export caches, index deterministic vector metadata for canonical summaries, and run file-vs-DB reconciliation with reader-projection parity diagnostics.
- Export audit paths now carry deterministic source-list hashes, replay export manifest metadata can be listed by reviewer/admin tokens, replay/benchmark/ranker export call sites mirror short-lived grant plus export-job lifecycle rows into the DB control plane, replay export jobs terminalize as failed when metadata or required object-ref body reads fail after job start, benchmark/ranker export jobs terminalize as failed for pre-publication metadata/source collection, source object-ref revalidation, and source-read audit failures, export-worker automation has dedicated replay/ranker routes plus CLI helpers, and admins can inspect durable export grants/jobs through tenant-scoped API/CLI list surfaces.
- Durable tenant access grant storage now exists for issuer-authorized principals and hosted-agent multitenant permissioning, including role, consent/use allow-lists, issuer/audience/subject attribution, expiry, revocation fields, tenant-scoped admin create/list/revoke APIs and CLI helpers, and tenant-scoped PostgreSQL RLS/libSQL predicates. When `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true`, trace submission and contributor credit/status readback are denied without an active exact-role grant for the authenticated principal, and grant scopes/uses narrow the effective request policy.
- Revocation, retention expiration, and maintenance-discovered file tombstones already invalidate DB-mirrored submission status, object refs, derived records, vector metadata, replay export manifests, and replay export item rows. Non-dry-run physical purge also marks only the exact deleted file/service-local submitted-envelope object refs with `deleted_at`.

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
Production-like deployments can require explicit tenant policy entries, ingest
can now read those policies from the TraceCorpusStore behind
`TRACE_COMMONS_DB_TENANT_POLICY_READS`, and worker tokens now have scoped
export, retention, vector, and benchmark route gates; fuller RBAC/ABAC remains
future work.
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
