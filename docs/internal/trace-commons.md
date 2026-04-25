# IronClaw Trace Commons

Trace Commons is an opt-in pipeline for contributing locally redacted IronClaw traces to a private corpus. It is separate from replay trace fixtures: replay traces support deterministic tests, while contribution envelopes carry consent, redaction metadata, replayability metadata, scoring, revocation, and contributor credit.

## Local-First Rules

- Trace contribution is off by default.
- Raw traces stay local.
- Uploads require a standing opt-in policy with an explicit ingestion endpoint.
- The client submits only `ironclaw.trace_contribution.v1` envelopes after deterministic local redaction.
- Message text and tool payloads remain excluded unless the user opts into those fields.
- Medium/high privacy risk traces can be held for manual review by policy.
- OpenAI Privacy Filter or other PII sidecars must only contribute safe summaries: redacted text, counts, labels, and warnings. Do not serialize original text or `detected_spans[*].text`.
- `safe_privacy_filter_redaction_from_output` converts Privacy Filter-style output to redacted text plus `SafePrivacyFilterSummary`, dropping raw `text` and raw span contents.
- Tool-specific structured redaction treats email, calendar, messaging, browser, filesystem, and database payload fields as sensitive before generic secret/path scrubbing.
- Deterministic text redaction preserves safe within-trace structure with stable placeholders such as `<PRIVATE_EMAIL_1>` and `<PRIVATE_LOCAL_PATH_1>` instead of flattening every entity to the same token.
- A local Privacy Filter sidecar can be enabled with `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` and optional whitespace-split `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS`. The sidecar receives `{"text":"..."}` on stdin and must return Privacy Filter-style JSON on stdout. IronClaw keeps only the safe `redacted_text` and aggregate summary.

## CLI MVP

```bash
# Enable autonomous submission after local redaction.
ironclaw traces opt-in \
  --endpoint https://trace-ingest.internal/v1/traces \
  --scope debugging-evaluation

# Create a local redacted envelope from an existing recorded trace.
ironclaw traces preview \
  --recorded-trace trace.json \
  --output contribution.json

# Queue a redacted envelope for autonomous submission.
ironclaw traces enqueue --envelope contribution.json

# Or preview and queue in one step.
ironclaw traces preview \
  --recorded-trace trace.json \
  --enqueue

# Submit eligible queued envelopes using the standing policy.
ironclaw traces flush-queue

# See local credit totals and recent explanations.
ironclaw traces credit

# Disable autonomous contribution.
ironclaw traces opt-out
```

The submit token is read from `IRONCLAW_TRACE_SUBMIT_TOKEN` by default. The token is not stored in the policy file.

## Private Ingestion Service MVP

The repository includes a local private-corpus ingestion binary for development and internal deployments:

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a,tenant-a:reviewer:review-token-a,tenant-b:dev-token-b' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run --bin trace_commons_ingest
```

Optional dark-launch storage can be enabled for internal pilots:

```bash
# Mirror corpus metadata into the configured IronClaw database.
TRACE_COMMONS_DB_DUAL_WRITE=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Optionally serve contributor credit/status endpoints from that DB mirror.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_CONTRIBUTOR_READS=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Optionally serve reviewer metadata views from that DB mirror.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_REVIEWER_READS=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Optionally select replay exports from DB metadata.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Optionally serve reviewer audit reads from the DB mirror.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_AUDIT_READS=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Store submitted redacted envelopes in the encrypted local artifact sidecar.
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_ARTIFACT_DIR=/var/lib/ironclaw/trace-artifacts \
cargo run --bin trace_commons_ingest
```

`TRACE_COMMONS_DB_DUAL_WRITE=true` builds a `TraceCorpusStore` mirror from the normal `DATABASE_BACKEND` configuration. `DATABASE_BACKEND=postgres` requires `DATABASE_URL`; `DATABASE_BACKEND=libsql` uses `LIBSQL_PATH` with optional `LIBSQL_URL` and `LIBSQL_AUTH_TOKEN`. The mirror writes tenant-scoped submissions, object refs, derived precheck records, audit events, credit events, review state, and revocation tombstones, including redaction-count aggregates and derived summary/tool/coverage metadata needed for DB-backed reviewer/export/analytics paths. By default, pilot API reads still use the file-backed store. `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true` switches `/v1/contributors/me/credit`, `/v1/contributors/me/credit-events`, and `/v1/contributors/me/submission-status` to the DB mirror; it requires DB dual-write/backfill to be configured and preserves tenant plus principal filtering. `TRACE_COMMONS_DB_REVIEWER_READS=true` switches reviewer/admin metadata reads for analytics, trace listing, quarantine queue, active-learning queue, benchmark candidate conversion, and ranker candidate/pair exports to the DB mirror. `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true` switches replay export eligibility and derived metadata selection to the DB mirror while still loading the redacted envelope body through the existing object path. `TRACE_COMMONS_DB_AUDIT_READS=true` switches `/v1/audit/events` to the DB mirror.

`TRACE_COMMONS_ARTIFACT_KEY_HEX` enables the encrypted artifact sidecar. `TRACE_COMMONS_ENCRYPTED_ARTIFACTS=true` can be used as an explicit guard, but still requires the key. When enabled, submitted redacted envelopes are encrypted with IronClaw secrets crypto, stored under a tenant-hashed artifact directory, and referenced by an `EncryptedTraceArtifactReceipt`. File-backed submission records retain the receipt so envelope reads resolve through the encrypted sidecar when present.

Then opt a client into that endpoint:

```bash
export IRONCLAW_TRACE_SUBMIT_TOKEN='dev-token-a'

ironclaw traces opt-in \
  --endpoint http://127.0.0.1:3907/v1/traces \
  --scope debugging-evaluation
```

The service exposes:

- `GET /health`
- `POST /v1/traces`
- `GET /v1/traces`
- `DELETE /v1/traces` with `{ "submission_id": "..." }`
- `DELETE /v1/traces/{submission_id}`
- `POST /v1/traces/{submission_id}/revoke`
- `GET /v1/contributors/me/credit`
- `GET /v1/contributors/me/credit-events`
- `POST /v1/contributors/me/submission-status`
- `GET /v1/analytics/summary`
- `GET /v1/review/quarantine`
- `POST /v1/review/{submission_id}/decision`
- `POST /v1/review/{submission_id}/credit-events`
- `GET /v1/datasets/replay`
- `POST /v1/benchmarks/convert`
- `POST /v1/admin/maintenance`
- `GET /v1/audit/events`

The ingestion service treats every upload as untrusted. It validates the schema and revocable consent, re-runs deterministic redaction on the submitted envelope, recomputes privacy hashes and credit estimates, stores accepted low-risk traces under the authenticated tenant, and quarantines medium/high-risk traces with zero immediate credit. Revocation writes a tenant-scoped tombstone and marks local metadata revoked.

`POST /v1/admin/maintenance` can also be used by reviewers/admins to bridge file-backed pilot data into the optional DB mirror. Set `backfill_db_mirror: true` to validate tenant-local file-backed submissions, envelopes, and derived precheck records, then mirror submissions that are not already present in the configured DB. Set `index_vectors: true` to publish deterministic canonical-summary vector metadata rows from accepted DB-mirrored derived records. Use `dry_run: true` to count valid backfill or vector-index candidates without writing DB rows.

Tenant tokens can be configured as either `tenant_id:token` for contributor access or `tenant_id:role:token` where role is `contributor`, `reviewer`, or `admin`. Each token is treated as its own pseudonymous auth principal inside the tenant. Reviewer/admin tokens can list tenant-local quarantine, approve or reject submissions, append delayed credit events, view tenant analytics, and export approved replay dataset slices. Contributor tokens can submit, revoke, read their own token-principal credit/events, and sync status for their known submission ids, but cannot review, view tenant-wide analytics, append credit events, or export datasets.

On submit, the service also writes a derived redacted-only record with:

- canonical summary and hash
- hash-based duplicate precheck
- placeholder novelty score for later vector replacement
- coverage tags for channel, tool, tool category, outcome, failure mode, and privacy risk
- aggregate analytics by status, privacy risk, task success, tool, tool category, and coverage tag

The current API remains intentionally file-backed under `TRACE_COMMONS_DATA_DIR` or `~/.ironclaw/trace_commons_ingest` for compatibility and easy local operation, with an optional DB-backed read path for contributor credit/status endpoints. This branch also includes the first production storage bridge: optional DB dual-write metadata and optional encrypted local artifact storage. Production deployments should finish promoting reviewer/export/analytics paths into DB/object-primary reads, add row-level tenant policies, and move encrypted artifacts behind service-owned object storage before broad rollout.

## Production Hardening Roadmap

The current implementation is a usable MVP for local development and controlled internal pilots. A production Trace Commons deployment needs the following before broad tenant rollout:

### DB and Object Storage

- Promote the current dual-write mirror into relational metadata reads for all API surfaces and service-owned encrypted object storage for redacted trace bodies. Contributor credit/status reads already have an opt-in DB-backed rollout gate.
- Keep metadata and object keys tenant-scoped from the auth-derived tenant id. Do not trust tenant fields in the envelope as storage partition keys.
- Store immutable submission records, append-only credit events, revocation tombstones, review decisions, export job manifests, and processing job state as separate records.
- Use row-level tenant policies or an equivalent authorization layer for every metadata query.
- Encrypt object storage at rest, require TLS in transit, and keep object bucket access behind service identities rather than reviewer/user tokens.
- Version all derived artifacts. A redaction, vector, ranking, benchmark, or export worker must record input envelope hash, worker version, policy version, and output artifact id.

### Tenant RBAC and ABAC

- Move beyond static tenant tokens before production. Prefer short-lived tokens or signed upload claims bound to tenant, actor, role, allowed scopes, and expiry.
- Enforce RBAC for contributor, reviewer, admin, trainer/export job, and service-worker roles.
- Add ABAC checks for consent scope, allowed use, privacy risk, review state, retention policy, revocation state, export purpose, and tenant data residency.
- Require privileged operations such as review override, bulk export, delayed credit mutation, retention override, and tombstone deletion to carry an explicit reason.
- Treat envelope contributor ids as pseudonymous attribution only. Authorization must come from request identity and central policy.

### Audit and Reviewability

- Add append-only audit events for every trace read, write, review decision, credit mutation, revocation, export, retention purge, and worker-derived artifact.
- Include tenant id, actor or job id, role, submission id, action, reason, request id, decision inputs, and output artifact ids.
- Make audit logs tamper-evident and queryable by tenant/security reviewers without exposing raw trace content.
- Add sampled audit reconciliation jobs that compare object storage, metadata rows, vector ids, export manifests, credit ledger rows, and revocation tombstones.

### Retention and Deletion

- Define retention windows by consent scope and allowed use. The envelope's `trace_card.retention_policy` should map to central policy, not directly drive deletion behavior.
- Implement retention jobs that remove or tombstone metadata, redacted trace objects, derived vectors, benchmark artifacts, export cache entries, and queued worker outputs.
- Keep revocation tombstones long enough to prevent re-ingest/re-export of the same submission hash after content deletion.
- Block new processing and export for revoked or expired submissions. Existing derived artifacts must be marked invalid before any downstream job consumes them.

### Revocation Propagation

- Treat revocation as a state transition that fans out to object storage, review queues, vector indexes, benchmark sets, ranking/training queues, export jobs, and credit ledgers.
- Make revocation idempotent and tenant-scoped. Repeated requests should preserve the first revocation reason/time unless an admin appends audit context.
- Require downstream workers to check central revocation state immediately before reading trace content and immediately before publishing a derived artifact.
- Add reconciliation that finds derived artifacts whose source submission is revoked and marks or removes them.

### Vector Index, Ranking, and Benchmark Conversion

- Generate embeddings only from redacted summaries and approved redacted trace fields. Never embed raw traces, sidecar raw text, or unreviewed high-risk content.
- Keep vector ids tenant-scoped and source-linked so index entries can be deleted or invalidated on revocation/retention.
- Replace placeholder novelty scoring with a private vector duplicate/novelty worker that records nearest neighbors, duplicate score, cluster id, and coverage contribution.
- Add ranking/model-utility jobs as offline analysis. Their outputs may append delayed credit events, but should not become immediate automatic payment signals.
- Convert approved traces into benchmark/replay datasets through a controlled job that records consent scope, review state, redaction version, deterministic replay requirements, and export manifest id.
- Require benchmark conversion to fail closed when the trace is revoked, expired, not approved for the target use, or missing replayability metadata.

### Privacy Filter Sidecar Operations

- Run Privacy Filter sidecars as untrusted local subprocesses or containers with timeouts, output size limits, and no access to Trace Commons credentials.
- Pass only the minimum text required for local redaction. Do not pass bearer tokens, full policy files, queue files, or raw tool payloads unless the local policy explicitly includes those fields.
- Accept only the safe projection: redacted text, labels, counts, warnings, and summary metadata. Strip `text`, raw span strings, offsets tied to raw text when not needed, and any unknown high-risk fields.
- Treat sidecar failures as non-fatal redaction warnings and fall back to deterministic local redaction rather than uploading raw content.
- Add canary-secret tests that feed synthetic credentials, local paths, tenant ids, and user ids through the sidecar path and assert they do not appear in envelopes, logs, or derived summaries.

## Autonomous Submission Policy

The local policy is stored under `~/.ironclaw/trace_contributions/policy.json` and controls:

- endpoint and bearer token environment variable
- default consent scope
- whether redacted message text or tool payloads may be included
- selected tool filters
- minimum local submission score
- whether medium-risk traces require manual review
- periodic credit notice interval for future UI/runtime notifications

The runtime can call the same queue and flush behavior later after a task completes. When `flush-queue` runs under an enabled policy, it submits eligible traces autonomously and prints a credit update when the configured notice interval has elapsed.

The agent runtime also schedules an autonomous post-turn contribution pass after a response is persisted or a turn fails. It reads the authenticated user's scoped policy, verifies the thread still belongs to that user, captures the most recent turns from durable conversation history, locally redacts the envelope, queues it, and flushes eligible queued envelopes. If the flush produces a due credit notice, the agent sends a status update back through the originating channel.

During each queue flush and before web credit/submission responses, the client asks the ingestion API for status updates for locally known submitted ids. The status endpoint is tenant-bound by the bearer token and returns only records from that tenant's namespace, so delayed reviewer credit can be reflected locally without allowing broad corpus enumeration.

In the authenticated web gateway, policy, queue, ledger, and revocation state are scoped under a hashed user/tenant directory rather than the global CLI policy. Envelopes carry a pseudonymous contributor id and a separate pseudonymous tenant scope reference, so the private ingestion service can attribute credit and enforce tenant boundaries without storing raw user ids in the trace body.

## Multitenant Permissioning

Trace contribution authorization must be derived from the authenticated request or runtime identity, not from fields inside a submitted envelope. Envelope fields such as `contributor.pseudonymous_contributor_id` and `contributor.tenant_scope_ref` are attribution/provenance metadata only.

For local capture:

- Web preview and autonomous runtime capture use the authenticated `user_id` as the trace scope.
- Conversation history is read through tenant ownership checks before a contribution envelope is built.
- Local policy, queue, submission history, revocation state, and credit records live under `trace_contributions/users/<hash>` for the authenticated user scope.
- The envelope includes a stable pseudonymous contributor id and a separate stable pseudonymous tenant scope reference. Neither includes the raw user id.

For the private ingestion service:

- The service should bind every request to a tenant from AuthN/AuthZ credentials, such as a tenant-scoped token, mTLS identity, or signed upload claim.
- It should reject requests where the authenticated tenant is not allowed to submit for the claimed tenant scope.
- Central metadata, credit ledger rows, revocation tombstones, privacy review queues, trace objects, and export jobs should all be keyed by the auth-derived tenant id plus the authenticated principal or contributor pseudonym.
- RBAC/ABAC policies should allow contributors to see only their own submissions and credit, reviewers to see quarantined/redacted traces for permitted tenants, and trainer jobs to read approved slices through controlled jobs.
- Audit logs should record tenant id, actor id/job id, submission id, access reason, and export target for every individual trace read or mutation.

With these rules, trace contributions can be correctly permissioned and attributed in a multitenant deployment: the trusted tenant binding comes from authentication and database row policy, while pseudonymous envelope metadata supports corpus analytics and credit assignment without becoming a trust boundary.

## Trace Commons Threat-Model Checklist

Use this checklist for any change touching trace capture, redaction, ingestion, review, export, credit, or derived datasets.

- Raw trace non-upload: verify raw recorded traces never leave the client; only `ironclaw.trace_contribution.v1` envelopes produced after local redaction may be submitted.
- Frontend untrusted: treat gateway UI requests as user-controlled input. Re-check auth, tenant ownership, policy scope, and conversation ownership on the server before previewing, queueing, submitting, listing, or revoking traces.
- Sidecar output stripping: reject or strip Privacy Filter sidecar fields that can carry original text, raw detected span text, raw offsets that are unnecessary downstream, or unknown nested payloads.
- Token isolation: submit/review/admin tokens must not be stored in policy files, trace envelopes, queue files, sidecar stdin, logs, or exported datasets.
- Tenant isolation: every ingestion read/write must bind to the auth-derived tenant and actor. Contributor-supplied `tenant_scope_ref`, `pseudonymous_contributor_id`, `submission_id`, and `revocation_handle` are not authorization inputs.
- Role isolation: contributors cannot list quarantine, append delayed credit, read analytics, export datasets, or probe other contributors' submissions. Reviewers/admins cannot bypass tenant scope.
- Bulk export controls: dataset export must require an authorized role, explicit purpose, consent/use filter, privacy-risk filter, review state filter, output manifest, and audit event per source trace.
- Delayed credit abuse: delayed credit append must be privileged, append-only, audited, bounded by policy, and linked to a concrete downstream artifact or review decision.
- Revocation propagation: revocation must block future status changes, review approval, vector indexing, benchmark conversion, ranking/training use, and export. Existing derived artifacts need invalidation or removal.
- Retention bypass: retention jobs must cover central metadata, object storage, vector entries, benchmark artifacts, export caches, worker queues, and local references where applicable.
- Canary secret tests: include synthetic API keys, bearer tokens, local paths, emails, tenant ids, user ids, and tool payload secrets in regression fixtures and assert none survive in accepted envelopes or sidecar-derived summaries.
- Audit completeness: any path that reads or mutates central trace content, credit, review state, export state, or revocation state must emit a tenant-scoped audit event.

Protected web API endpoints:

- `GET /api/traces/policy`
- `PUT /api/traces/policy`
- `POST /api/traces/preview`
- `POST /api/traces/submit`
- `POST /api/traces/flush`
- `GET /api/traces/credit`
- `GET /api/traces/submissions`
- `POST /api/traces/submissions/{submission_id}/revoke`

The web settings panel includes a Trace Commons tab for standing opt-in, autonomous submission controls, queue flushing, recent submissions, revocation, and credit totals. The chat composer also has a Trace button that previews the current thread's redacted envelope and can queue it for the same autonomous submission path.

## Implementation Status Matrix

| Area | Status | Maintainer notes |
|------|--------|------------------|
| Local opt-in policy and opt-out | Implemented MVP | CLI and scoped web/runtime policy files exist; submit token stays in environment. |
| Local preview, queue, flush, and credit display | Implemented MVP | CLI and web paths use local redacted envelopes and local submission metadata. |
| Deterministic local redaction | Implemented MVP | Includes generic secret/path scrubbing, stable placeholders, tool-aware payload handling, and Privacy Filter safe projection. |
| Privacy Filter sidecar integration | Implemented MVP | Local command/stdin/stdout path exists; production sandboxing, canary tests, and stricter output contracts remain. |
| Autonomous post-turn contribution | Implemented MVP | Runtime queues/flushed scoped envelopes after persisted or failed turns when policy permits. |
| Web Trace Commons settings and preview endpoints | Implemented MVP | Authenticated gateway endpoints and UI controls exist; server-side tenant/user checks remain the trust boundary. |
| Private ingestion service | Implemented MVP | Development/internal binary validates schema, reruns redaction, computes hashes/credit, stores accepted/quarantined records, and serves review/status/export routes. It can now dark-launch DB dual-write metadata and encrypted envelope artifacts. |
| Tenant token roles | Partial | Static tenant tokens support contributor/reviewer/admin behavior. Production needs short-lived credentials, central policy, RBAC/ABAC, and row-level tenant enforcement. |
| Contributor credit ledger and delayed credit sync | Partial | Append-only local and central credit events exist, and autonomous clients periodically notify opted-in users when submitted or later-revoked records receive ledger changes. Production needs stricter privilege policy, artifact linkage, anti-abuse review, and audit reconciliation. |
| Quarantine/review workflow | Partial | Reviewer/admin routes can list and decide on quarantined redacted traces. Production needs durable DB state, audit, assignment, escalation, and retention/revocation gates. |
| Replay dataset export | Partial | Approved redacted slices can be exported by reviewer/admin tokens, and DB metadata can now drive replay export selection when envelope objects remain readable. Production needs object-primary reads, bulk export controls, manifests, per-trace audit, retention filters, and revocation invalidation. |
| Analytics summary | Partial | Aggregate counts by status/risk/tool/coverage exist. Production needs tenant policy, privacy budgets if exposed broadly, and audit for privileged analytics. |
| Production relational DB and encrypted object storage | Partial | V25/V26/V27/V28 PostgreSQL/libSQL schema, shared `TraceCorpusStore`, both backend implementations, optional ingest DB mirror with contributor, reviewer metadata, replay selection reads, vector-entry metadata, encrypted local artifact sidecar, and maintenance-triggered DB mirror backfill exist. Object-primary replay/export reads, full reconciliation, RLS/policy enforcement, and service-owned object storage remain. |
| Central audit log | Partial | File-backed audit routes and optional DB audit reads cover core submit/review/credit/revoke mutations plus reviewer trace-list reads and dataset/benchmark/ranker exports. Production still needs tamper-evident audit, per-trace content-read coverage, reconciliation, and privileged reason enforcement. |
| Retention enforcement | Not implemented | Envelope records policy metadata, but central purge/tombstone propagation jobs are still needed. |
| Revocation propagation to derived artifacts | Partial | Current revocation marks local/file status, mirrors DB status, writes tenant-scoped tombstones, invalidates DB-mirrored object refs, derived precheck rows, and vector metadata entries, and blocks file-backed replay export. Production must invalidate benchmark, ranking, worker, and existing export artifacts. |
| Vector duplicate/novelty index | Partial | DB schema, storage contract, and maintenance-triggered metadata indexer now persist vector-entry metadata, nearest trace ids, cluster id, duplicate score, novelty score, and invalidation state for accepted canonical summaries. The embedding worker/vector payload index is still future work. |
| Ranking/model utility pipeline | Not implemented | Delayed credit kinds are reserved; no trusted offline utility job is implemented. |
| Benchmark conversion pipeline | Not implemented | Replay export exists, but controlled benchmark artifact conversion with manifests and consent gates is still future work. |
| Production sidecar operations | Not implemented | Needs sandboxing, resource limits, log hygiene, output schema enforcement, and canary-secret regression coverage. |

## Credit

The client computes a local pending credit estimate from a trace value scorecard. The scorecard keeps privacy risk, quality, replayability, capped novelty, duplicate penalty, coverage, difficulty, dependability, and correction value as separate components before producing the online score.

Each local submission record can store append-only credit events. The initial event records accepted submission credit; delayed events from benchmark conversion, regression catches, reviewer value, ranking/model utility, or abuse penalties should be appended later by the private ingestion pipeline. Shapley-style or influence estimates can inform offline analysis, but should not be exposed as direct immediate payment logic.

The ingestion API can return a receipt with updated pending/final credit and explanations; those values are stored locally in `submissions.json`.

Delayed credit/status refresh uses:

```http
POST /v1/contributors/me/submission-status
Authorization: Bearer <tenant-token>

{ "submission_ids": ["..."] }
```

The response is an array of records visible to that authenticated principal. Missing ids are omitted, which keeps cross-tenant and same-tenant cross-principal probes indistinguishable from unknown submissions.

Status records include the base submission credit plus delayed ledger fields when review or downstream jobs have awarded later utility credit: `credit_points_ledger`, `credit_points_total`, and `delayed_credit_explanations`. Local autonomous clients store the total as the effective final credit and reset the credit-notice timer, so opted-in users can be periodically informed about benchmark, regression, ranking/training, reviewer, or abuse-penalty adjustments without seeing other contributors' ledger rows.

Reviewers/admins can append delayed credit after downstream utility is known:

```http
POST /v1/review/{submission_id}/credit-events
Authorization: Bearer <reviewer-token>

{
  "kind": "benchmark_use_bonus",
  "points": 2.5,
  "explanation": "Converted into a trajectory benchmark"
}
```

Contributors can read only their own append-only central credit events:

```http
GET /v1/contributors/me/credit-events
Authorization: Bearer <tenant-token>
```

Users can inspect their local ledger with:

```bash
ironclaw traces credit
ironclaw traces list-submissions
```

## Research Hooks

The MVP envelope intentionally reserves fields for later processing without implementing the whole central pipeline:

- `trace_card` documents consent scope, allowed uses, source channel, tool categories, retention, and revocation.
- `value_card` documents the score version, full scorecard, limitations, and user-visible credit explanation.
- `embedding_analysis` stores canonical summary hashes, vector IDs, nearest traces, clusters, duplicate score, novelty score, and coverage tags once a private worker fills them.
- `hindsight` keeps failed traces useful by allowing later subgoal/recoverability labels.
- `training_dynamics` supports future dataset cartography labels such as easy, ambiguous, or hard.
- `canonical_summary_for_embedding` builds redacted-only summaries for embedding and duplicate detection.
