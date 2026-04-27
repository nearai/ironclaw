# IronClaw Trace Commons

Trace Commons is an opt-in pipeline for contributing locally redacted IronClaw traces to a private corpus. It is separate from replay trace fixtures: replay traces support deterministic tests, while contribution envelopes carry consent, redaction metadata, replayability metadata, scoring, revocation, and contributor credit.

## Local-First Rules

- Trace contribution is off by default.
- Raw traces stay local.
- Uploads require a standing opt-in policy with an explicit ingestion endpoint.
- The client submits only `ironclaw.trace_contribution.v1` envelopes after deterministic local redaction.
- Message text and tool payloads remain excluded unless the user opts into those fields.
- Medium/high privacy risk traces can be held for manual review by policy.
- OpenAI Privacy Filter or other PII sidecars must only contribute safe summaries: redacted text, allow-listed label counts, and warnings. Do not serialize original text or `detected_spans[*].text`.
- `safe_privacy_filter_redaction_from_output` converts Privacy Filter-style output to redacted text plus `SafePrivacyFilterSummary`, dropping raw `text`, raw span contents, raw offsets, and unsafe span labels.
- Tool-specific structured redaction treats email, calendar, messaging, browser, filesystem, and database payload fields as sensitive before generic secret/path scrubbing.
- Deterministic text redaction preserves safe within-trace structure with stable placeholders such as `<PRIVATE_EMAIL_1>` and `<PRIVATE_LOCAL_PATH_1>` instead of flattening every entity to the same token.
- A local Privacy Filter sidecar can be enabled with `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` and optional whitespace-split `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS`. The sidecar receives `{"text":"..."}` on stdin and must return Privacy Filter-style JSON on stdout. IronClaw keeps only the safe `redacted_text` and aggregate summary. The sidecar is launched with a cleared environment except `PATH`, `LANG`, and `LC_ALL`; `IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS`, `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES`, `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES`, and `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES` tune local guardrails.

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

# Acknowledge or snooze a due periodic credit notice.
ironclaw traces credit --notice --ack
ironclaw traces credit --notice --snooze-hours 24

# Disable autonomous contribution.
ironclaw traces opt-out
```

The submit token is read from `IRONCLAW_TRACE_SUBMIT_TOKEN` by default. The token is not stored in the policy file. `preview --enqueue` and `enqueue --envelope` use the same standing-policy gate as autonomous flush: the policy must be enabled, must have an ingestion endpoint, and must allow any message text or tool payloads already present in the redacted envelope. Plain `preview` remains local and does not require opt-in.

## Private Ingestion Service MVP

The repository includes a local private-corpus ingestion binary for development and internal deployments:

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a;expires_at=2026-04-27T00:00:00Z,tenant-a:reviewer:review-token-a,tenant-a:export_worker:export-token-a,tenant-b:dev-token-b' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run --bin trace_commons_ingest
```

Token entries may use `tenant_id:token` for contributor access or
`tenant_id:role:token` for scoped roles. Either form may append a short-lived
credential expiry attribute, for example
`tenant_id:role:token;expires_at=2026-04-27T00:00:00Z` or
`tenant_id:token;expires=2026-04-27T00:00:00Z`. Expired bearer tokens are
rejected before tenant attribution, and token-principal hashes are computed from
the secret token value only, not the expiry metadata. The ingest service recognizes
`contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`,
`vector_worker`, `benchmark_worker`, `utility_worker`, and
`process_eval_worker` (also accepted as `process_evaluation_worker` for token
configuration). Worker roles do not inherit reviewer
visibility: export workers can build replay/ranker exports, benchmark workers
can run benchmark conversion through either the reviewer-compatible conversion
route or a dedicated worker route, retention workers can run the dedicated
retention worker route or retention-scoped maintenance, and vector workers can
run the dedicated vector-index worker route or vector-index maintenance. Utility
credit workers can append idempotent delayed utility credit through their
dedicated route for accepted traces only, without access to reviewer bonus,
abuse penalty, review queues, audit logs, or tenant policy administration.
As an alternative to configured static bearer tokens, internal deployments can
set `TRACE_COMMONS_SIGNED_TOKEN_SECRET` to accept HS256 signed tenant claims.
This HS256 path is an internal bridge for controlled pilots, not the production
asymmetric-token design. The current service also accepts EdDSA/Ed25519 signed
claims when configured with `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_PEM`
or `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILE`; keyed EdDSA public-key
rotation can use `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILES` as
comma-separated `kid:path` entries. Production upload claims should be treated
as EdDSA/Ed25519-only: static tokens and HS256 signed claims remain bridge paths
for controlled pilots, not the production claim mechanism. The verifier rejects
unsupported JWT algorithms and can run with only EdDSA keys configured.
For HMAC bridge rotation, `TRACE_COMMONS_SIGNED_TOKEN_SECRETS` can also provide
comma-separated `kid:secret` entries; signed tokens with a JWT header `kid`
must match one of those configured key ids, while tokens without `kid` use the
single default secret when present. EdDSA keyed public-key files use the same
JWT `kid` selection behavior. For activation-window rotation,
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_JSON` or
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_FILE` can load a JSON keyset with
entries shaped as `{ "kid": "...", "public_key_pem": "...", "not_before":
"<RFC3339>", "not_after": "<RFC3339>" }`; `not_before` and `not_after` are
optional, but inactive keys are rejected before token verification.
Config status reports the total signed-token key count and the EdDSA key count,
not key material or key ids.
Signed tokens must include `tenant_id`, `exp`, and either `principal_ref` or
`sub`; `role` defaults to `contributor` and may use the same role names as
static tokens. Claims may also include `allowed_consent_scopes` and
`allowed_uses` arrays to restrict which submitted trace consent scopes and
trace-card uses the bearer token can upload or later use for replay exports,
benchmark/ranker dataset generation, process-evaluation labeling, and utility
credit. Set
`TRACE_COMMONS_SIGNED_TOKEN_ISSUER` and/or
`TRACE_COMMONS_SIGNED_TOKEN_AUDIENCE` to require matching `iss` and `aud`
claims. Set `TRACE_COMMONS_SIGNED_TOKEN_MAX_TTL_SECONDS` to require `iat` and
reject tokens whose `exp - iat` lifetime exceeds the configured bound. Set
`TRACE_COMMONS_SIGNED_TOKEN_REQUIRE_JTI=true` to require every signed claim to
carry a JWT ID, and `TRACE_COMMONS_SIGNED_TOKEN_REVOKED_JTIS` to a
comma-separated emergency denylist of JWT `jti` values. The config-status route
reports only whether signed-token auth and issuer/audience checks are enabled
plus the revoked-`jti` count, max TTL, and require-`jti` flag; it never returns
the secret or configured claim values.
Submitted-trace audit rows record only the safe auth method (`static_token` or
`signed_claim`) plus the hashed actor principal.
Local/file managed EdDSA keysets with activation windows are available for
controlled deployments. Remaining issuer work is distribution and governance:
fetching or syncing issuer-owned Ed25519 key records, surfacing active/inactive
key diagnostics safely, and making the issuer keyset the only production
upload-claim rotation path.
Process-evaluation workers can submit bounded process quality metadata through
`POST /v1/workers/process-evaluation` using the CLI helper:

```bash
ironclaw traces process-evaluation-submit \
  --endpoint https://trace-ingest.internal/v1/traces \
  --submission-id 018f2b7b-0c11-72fd-95c4-1f9f98feac01 \
  --reason "nightly evaluator pass" \
  --evaluator-version process-eval-2026-04-26 \
  --label proper_verification \
  --tool-selection pass \
  --verification partial \
  --utility-credit-points-delta 0.5 \
  --utility-external-ref process-eval:nightly:2026-04-26
```

When `--utility-credit-points-delta` is set, the helper also sends a trimmed
`utility_external_ref`; the service uses that external reference to append an
idempotent `training_utility` delayed credit event for the evaluated accepted
submission. Non-JSON output prints appended/skipped credit counts when the
server returns them.

Internal deployments can also add a fail-closed tenant submission policy. When
`TRACE_COMMONS_TENANT_POLICIES` contains an entry for the authenticated tenant,
new submissions must use only the listed consent scopes and trace-card allowed
uses before the server re-scrubs and stores them:

```bash
TRACE_COMMONS_TENANT_POLICIES='{
  "tenant-a": {
    "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
    "allowed_uses": ["debugging", "evaluation", "benchmark_generation", "aggregate_analytics"]
  }
}' \
cargo run --bin trace_commons_ingest
```

Tenants without an explicit entry keep the development default so existing local
pilots continue to work. When a policy exists, it is also used as downstream
ABAC: replay exports require the `evaluation` use, benchmark conversion requires
`benchmark_generation`, ranker candidate/pair exports require
`ranking_model_training`, and vector indexing requires at least one derived-use
permission (`debugging`, `evaluation`, `benchmark_generation`,
`ranking_model_training`, or `model_training`). Matching source traces must
carry an allowed consent scope and the required trace-card allowed use, so
pre-policy records without matching allowed-use metadata are skipped. The
aggregate-only use is intentionally insufficient for vector indexing because
that retention class does not permit derived artifacts. Production deployments
should configure this policy for every tenant and treat envelope contributor
fields as attribution only. Set
`TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY=true` to reject new submissions
and export requests from tenants that do not have an explicit policy entry.

Set `TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS=true` in production-like ingestion
deployments to require explicit low-risk, accepted-status, consent-scoped replay
and benchmark export filters. Ranker training exports also require explicit
accepted-status, low-risk, ranking/model-training consent filters when this
guard is enabled.
Set `TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST` to lower the per-request item
cap for replay, benchmark, and ranker exports; the default remains 500 for
compatibility, and requests above the configured cap are clamped.

Set `TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR` and/or
`TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR` to bound autonomous
contributor uploads. Quotas are disabled by default, apply only to contributor
tokens, count active accepted/quarantined submissions in the last hour, and do
not block idempotent retries of an existing submission id. Revoked, expired, and
purged submissions stop consuming quota.

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
# Production-like rollouts can add TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true.
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

# Fail closed when benchmark/ranker export sources lack active submitted-envelope object refs.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Optionally serve reviewer audit reads from the DB mirror.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_AUDIT_READS=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Fail maintenance closed when DB/file reconciliation reports promotion-blocking gaps.
# Use with admin maintenance requests that set reconcile_db_mirror: true.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Pause retention expiration/purge for selected central retention policy IDs.
TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES=private_corpus_revocable,benchmark_revocable \
cargo run --bin trace_commons_ingest

# Store submitted redacted envelopes in the encrypted local artifact sidecar.
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_ARTIFACT_DIR=/var/lib/ironclaw/trace-artifacts \
cargo run --bin trace_commons_ingest

# Prefer the service-owned local object-store backend for production-shaped pilots.
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
cargo run --bin trace_commons_ingest

# Skip plaintext submitted/reviewed envelope body files for submit/review pilots.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true \
TRACE_COMMONS_DB_REVIEWER_READS=true \
TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true \
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Require replay export body reads through service-local DB object refs.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true \
TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true \
TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true \
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest

# Skip plaintext benchmark/ranker artifact and provenance files.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true \
TRACE_COMMONS_DB_REVIEWER_READS=true \
TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true \
TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS=true \
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
DATABASE_BACKEND=libsql \
LIBSQL_PATH=/var/lib/ironclaw/ironclaw.db \
cargo run --bin trace_commons_ingest
```

`TRACE_COMMONS_DB_DUAL_WRITE=true` builds a `TraceCorpusStore` mirror from the normal `DATABASE_BACKEND` configuration. `DATABASE_BACKEND=postgres` requires `DATABASE_URL`; `DATABASE_BACKEND=libsql` uses `LIBSQL_PATH` with optional `LIBSQL_URL` and `LIBSQL_AUTH_TOKEN`. The mirror writes tenant-scoped submissions, tenant policies, object refs, derived precheck records, export manifest metadata, export manifest item snapshots, audit events, credit events, review state, revocation tombstones, and retention maintenance job/item ledger rows, including redaction-count aggregates and derived summary/tool/coverage metadata needed for DB-backed reviewer/export/analytics paths. By default, pilot API reads still use the file-backed store. `TRACE_COMMONS_DB_TENANT_POLICY_READS=true` switches submission and export policy lookup to DB-backed `trace_tenant_policies`; combine it with `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY=true` to fail closed when no tenant policy exists. Admin-token reads and writes through `/v1/admin/tenant-policy` append hash-chained file audit events and mirror safe DB audit metadata with policy version, allow-list counts, and a policy projection hash. Admin-token reads through `/v1/admin/config-status` expose only safe cutover booleans, schema version, DB/object-store configured status, configured legal-hold policy IDs, the max export item cap, submission quota limits, and the object-store provider alias; the response omits roots, tokens, paths, secrets, and tenant policy contents while still writing a read audit event. `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true` switches `/v1/contributors/me/credit`, `/v1/contributors/me/credit-events`, and `/v1/contributors/me/submission-status` to the DB mirror; it requires DB dual-write/backfill to be configured and preserves tenant plus principal filtering. `TRACE_COMMONS_DB_REVIEWER_READS=true` switches reviewer/admin metadata reads for analytics, trace listing, quarantine queue, active-learning queue, benchmark candidate conversion, ranker candidate/pair exports, review decisions, and review lease claim/release state to the DB mirror. Review leases are tenant-scoped, bound to the authenticated reviewer/admin principal, may be reclaimed by the same principal or after expiration, and are cleared automatically when a trace leaves quarantine. Review decisions resolve envelope bodies through active DB object refs first; compatibility mode may fall back to a file-backed body only when file metadata is still present, while DB-sourced submissions with no file metadata require an active submitted-envelope object ref and do not recreate the missing file metadata row. Set `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true` with DB reviewer reads to make all DB-backed review decisions fail closed when no active submitted-envelope object ref exists. `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true` switches replay export eligibility and derived metadata selection to the DB mirror, then attempts to resolve submitted envelope bodies through a shared replay body-read policy/audit helper that verifies tenant scope, object ref ownership, artifact kind, and content hash for DB object refs, including the encrypted local artifact sidecar. Compatibility mode falls back to the file-backed envelope body if no active DB object ref exists. Set `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true` with DB replay export reads to fail closed instead. `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true` requires DB dual-write and makes benchmark conversion plus ranker candidate/pair exports fail closed unless every selected source has an active submitted-envelope object ref that can be tenant/hash verified before artifacts, provenance, or utility credit are published. `TRACE_COMMONS_DB_AUDIT_READS=true` switches `/v1/audit/events` to the DB mirror. Maintenance reconciliation reports reader-projection parity for contributor credit/status/events, reviewer metadata, analytics, audit event counts, and replay/export manifest summaries so operators can check each read flag before promotion; it also reports file/DB credit-ledger and canonical audit-event ID gaps so operators can distinguish count parity from missing mirrored rows. Reconciliation responses include `blocking_gaps`, a compact machine-readable list of promotion blockers. Set `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` after dual-write/backfill parity looks stable to reject maintenance requests that omit `reconcile_db_mirror: true` with `400 Bad Request` and to fail closed with `409 Conflict` when reconciliation still reports promotion-blocking gaps. Requests that ask for reconciliation without a configured DB mirror return `503 Service Unavailable`. Audit-chain verification also includes a DB mirror report that checks previous-hash continuity, recomputes hashes for canonical mirrored payloads, and compares DB action/metadata projections against those payloads; derived diagnostics compare file/DB presence, status, canonical-summary hashes, and active derived rows attached to invalid sources; export diagnostics split DB manifests into replay, benchmark, ranker, and other counts, flag manifest items missing source object refs, and report active export manifests/items still referencing invalid sources; object-ref diagnostics distinguish missing/unreadable bodies from content-hash integrity mismatches; and vector diagnostics flag accepted/current canonical summaries that still need active vector entries.

Set `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` during production cutover after DB dual-write parity checks pass. It requires `TRACE_COMMONS_DB_DUAL_WRITE=true` and makes submission, revocation, review decision, credit, replay export manifest, benchmark/ranker provenance, audit, and trace-content-read mirror failures return an internal error instead of silently continuing with file-only state. Submission, delayed-credit, benchmark provenance, and ranker provenance mirror failures also avoid publishing local file-side metadata/object, ledger, artifact/provenance, or staged service-local encrypted artifact objects for the failed operation.

`TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES` is a comma-separated list of validated central retention policy IDs. Matching submissions are not newly expired or purged by maintenance even when `expires_at` or `purge_expired_before` would otherwise match; `/v1/admin/config-status` exposes the configured IDs for admin verification.

`TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` optionally suppresses aggregate analytics cells whose count is below the configured threshold. The endpoint still returns content-free totals and reports `min_cell_count` plus `suppressed_cell_count` so reviewers know a privacy threshold was applied.

`TRACE_COMMONS_ARTIFACT_KEY_HEX` enables encrypted trace object storage. `TRACE_COMMONS_ENCRYPTED_ARTIFACTS=true` can be used as an explicit guard for the legacy encrypted artifact sidecar, but still requires the key. `TRACE_COMMONS_OBJECT_STORE=local_service` selects the production-shaped service-owned local backend and records DB object refs with the `trace_commons_service_local_encrypted` provider alias. That mode uses `TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR` when set, otherwise `TRACE_COMMONS_ARTIFACT_DIR`, otherwise `TRACE_COMMONS_DATA_DIR/service_object_store`. In both encrypted modes, submitted redacted envelopes, benchmark conversion artifacts, and ranker export provenance manifests are encrypted with IronClaw secrets crypto, stored under a tenant-hashed artifact directory, and referenced by DB object refs. File-backed submission records retain envelope receipts so envelope reads resolve through encrypted object storage when present; benchmark/ranker export manifest items carry per-source object refs to the shared export artifact or provenance object. `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true` is a production-shaped submit/review cutover guard: it requires DB dual-write, required DB mirror writes, DB reviewer reads, reviewer object-ref reads, and `TRACE_COMMONS_OBJECT_STORE=local_service`, then omits the plaintext submitted/reviewed envelope body files while still writing compatibility metadata, derived records, and file audit rows. `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true` is the replay-export companion guard: it requires DB dual-write, required DB mirror writes, DB replay export reads, replay object-ref-required reads, and the service-local object store, then makes replay body exports use the existing DB object-ref path without file fallback. `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true` is the benchmark/ranker companion guard: it requires DB dual-write, required DB mirror writes, DB reviewer reads, required derived source object refs, export guardrails, and the service-local object store, then stores benchmark artifacts and ranker provenance only in encrypted object storage while keeping DB manifest/items as the durable index for purpose filters and lifecycle invalidation.

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
- `GET /v1/traces` with reviewer filters for status, privacy risk, consent scope, derived tool/tag metadata, and export/provenance `purpose`
- `DELETE /v1/traces` with `{ "submission_id": "..." }`
- `DELETE /v1/traces/{submission_id}`
- `POST /v1/traces/{submission_id}/revoke`
- `GET /v1/contributors/me/credit`
- `GET /v1/contributors/me/credit-events`
- `POST /v1/contributors/me/submission-status`
- `GET /v1/analytics/summary`
- `GET /v1/review/quarantine`
- `GET /v1/review/active-learning`
- `POST /v1/review/{submission_id}/decision`
- `POST /v1/review/{submission_id}/lease`
- `DELETE /v1/review/{submission_id}/lease`
- `POST /v1/review/{submission_id}/credit-events`
- `GET /v1/datasets/replay`
- `GET /v1/datasets/replay/manifests`
- `POST /v1/benchmarks/convert`
- `POST /v1/benchmarks/{conversion_id}/lifecycle`
- `POST /v1/workers/benchmark-convert`
- `POST /v1/workers/utility-credit`
- `GET /v1/ranker/training-candidates`
- `GET /v1/ranker/training-pairs`
- `GET|POST|PUT /v1/admin/tenant-policy`
- `GET /v1/admin/config-status`
- `POST /v1/admin/maintenance`
- `POST /v1/workers/retention-maintenance`
- `POST /v1/workers/vector-index`
- `GET /v1/audit/events`

The ingestion service treats every upload as untrusted. It validates the schema and revocable consent, re-runs deterministic redaction on the submitted envelope, recomputes privacy hashes and credit estimates, enforces optional per-tenant/per-principal hourly contributor submission quotas for autonomous upload safety, stores accepted low-risk traces under the authenticated tenant, and quarantines medium/high-risk traces with zero immediate credit. Reviewer quarantine and active-learning queues are prioritized work queues: queue items expose `review_age_hours`, `review_escalation_state`, `review_escalation_reasons`, and optional DB-backed review lease metadata so operators can filter by `lease_filter=all|mine|available|active|expired`, then sort by reviewer SLA pressure, assignment, and escalation causes without opening trace bodies. Revocation writes a tenant-scoped, first-writer-wins tombstone with redaction and canonical-summary hashes when available, marks local or DB-backed metadata revoked only for authorized owners/reviewers, lifecycle-revokes any published benchmark artifact derived from the source, and blocks later uploads in the same tenant that match a retained file-backed or DB-mirrored tombstone redaction hash, canonical-summary hash, or submission id.

Replay dataset exports, benchmark conversion artifacts, and ranker training exports include a deterministic `sha256:` hash of their source item list. The same hash is mirrored into audit `decision_inputs_hash` for the export event, giving reviewers a stable tenant-scoped proof of which submissions fed an exported dataset without exposing trace content in the audit row. Benchmark conversion artifacts also carry a schema version plus registry/evaluation lifecycle metadata; new conversions start as registry `candidate` and evaluation `not_run`, and benchmark workers or reviewers can update registry/evaluator state through the lifecycle endpoint without changing the export contract. The `ironclaw traces benchmark-lifecycle-update` helper posts the same registry/evaluator patch shape for worker automation and manual reviewer operations. Lifecycle updates rewrite the tenant-scoped file artifact in compatibility mode or the encrypted service-local object in object-primary mode, refresh DB object refs when the DB mirror is configured, and append an audit event with the registry/evaluation statuses. Benchmark and ranker exports also write tenant-local provenance manifests with source ids, caller-supplied purpose, and invalidation fields; CLI ranker candidate/pair export commands accept `--purpose` so guarded services can require it. Revocation and retention maintenance mark those provenance manifests invalid instead of deleting them. Benchmark conversion plus ranker candidate and pair exports revalidate DB source status immediately before publishing artifacts and append idempotent delayed utility credit events for included accepted sources, keyed by tenant plus submission so rerunning the same worker surface does not double-credit. Trusted offline utility jobs can use `POST /v1/workers/utility-credit` with `regression_catch`, `training_utility`, or `ranking_utility`, a concrete `external_ref`, and one or more accepted submission ids to append the same idempotent delayed credit without exposing reviewer-only credit mutation; the worker rejects sources whose consent/use metadata or tenant policy does not permit the requested evaluation, model-training, or ranking-training use. Tenant policy allowed-use ABAC is applied before exports and process-evaluation labeling, then again while selecting sources, so replay exports, benchmark conversion, ranker exports, and process-evaluation workers cannot publish or label sources outside the tenant's consent/use allow-list. Signed-token allow-lists are enforced on the same downstream paths: replay exports, benchmark/ranker dataset generation, process-evaluation workers, and utility-credit workers cannot consume sources outside the caller claim's allowed scopes/uses. Production-like deployments can also require active submitted-envelope object refs for every benchmark/ranker source so stale or unreadable trace bodies fail before publishing derived artifacts. Replay dataset exports mirror compact DB manifest rows plus per-source item snapshots with source status, hash, and source object ref at export time; benchmark and ranker item rows name the derived summary artifact version used and now carry per-source object refs to the benchmark artifact or ranker provenance payload. In file-backed mode those refs point at the local JSON artifact; in service-local encrypted mode they point at the tenant-checked encrypted object-store payload. Manifest and item rows are invalidated when any source submission is revoked, expired, or purged. Reviewer/admin tokens can inspect replay export manifest metadata through `GET /v1/datasets/replay/manifests`; DB-backed listing is scoped to replay dataset manifests and filters out benchmark/ranker provenance rows.

Vector indexing now follows the same downstream ABAC model: tenant policy and
signed-claim allowed-use filters are enforced before the worker runs, then both
the indexed source set and nearest-neighbor pool are reduced to traces that
permit a derived vector use. Aggregate-only traces are skipped because that
retention class does not permit derived artifacts.

`POST /v1/admin/maintenance` can also be used by reviewers/admins to bridge file-backed pilot data into the optional DB mirror. It marks submissions expired when their retention-policy `expires_at` has passed, mirrors expiration status plus artifact/export-manifest invalidation to the DB mirror when configured, writes a durable DB retention job row plus per-submission lifecycle item rows for mirrored expire/purge/revoke actions, repairs DB revocation/artifact invalidation for submissions that are already file-marked revoked, lifecycle-revokes published benchmark artifacts whose sources are revoked or expired, prunes cached export payloads whose manifest references revoked or expired sources, and keeps expired traces out of replay, benchmark, and ranker exports. Admin tokens can inspect the durable tenant-scoped retention ledger through `GET /v1/admin/retention/jobs` and `GET /v1/admin/retention/jobs/{retention_job_id}/items`; both routes require the configured DB mirror, support bounded filtered reads, and append read-audit breadcrumbs without exposing trace bodies. The matching CLI helpers are `ironclaw traces retention-jobs-list` and `ironclaw traces retention-job-items`, and the web Trace Commons operator panel exposes the same read-only job/item lookups with its session-only admin token. Retention workers should prefer the narrower `POST /v1/workers/retention-maintenance` route, which exposes only `purpose`, `dry_run`, `prune_export_cache`, `max_export_age_hours`, and `purge_expired_before`. Set `purge_expired_before` to an explicit RFC3339 cutoff plus a non-empty `purpose` to mark already-expired submissions purged and delete their file-backed and encrypted local artifact copies; dry-run purge previews may omit purpose. Set `backfill_db_mirror: true` to validate tenant-local file-backed submissions, envelopes, and derived precheck records, then mirror submissions that are not already present in the configured DB. Backfill isolates per-submission and per-export failures so one corrupt envelope, missing derived precheck, or unreadable replay manifest does not block valid records; the maintenance response and audit metadata include `benchmark_artifacts_invalidated` and `db_mirror_backfill_failed` plus bounded failure details. Set `index_vectors: true` to publish deterministic canonical-summary vector metadata rows from accepted DB-mirrored derived records; vector workers should prefer the narrower `POST /v1/workers/vector-index` route, which accepts only `purpose` and `dry_run` and internally builds a vector-only maintenance request. Non-dry-run vector indexing requires an active submitted-envelope object ref that can be read and hash-checked before the vector row is written, and mirrors the source object ref into the per-source content-read audit row. Set `reconcile_db_mirror: true` to return a tenant-scoped report comparing file-backed metadata counts, DB object/vector/export/tombstone counts, DB retention job/item counts, the just-written retention maintenance ledger row/item count, credit-ledger and canonical audit-event ID gaps, active submitted-envelope object ref presence/readability/hash integrity, export manifest item object-ref gaps, active derived/export rows that still point at invalid sources, reader-projection parity for contributor, reviewer metadata, analytics, audit, and replay/export manifest surfaces, and compact `blocking_gaps`. With `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true`, maintenance requests that omit `reconcile_db_mirror: true` fail before maintenance side effects with `400 Bad Request`, and reconciliation requests fail with `409 Conflict` until `blocking_gaps` is empty. Failed dirty reconciliation attempts still append the normal maintenance audit event after any completed maintenance work. Set `verify_audit_chain: true` to include a file-backed audit hash-chain integrity report plus, when a DB mirror is configured, a DB audit report with canonical-payload hash recomputation and projection checks for rows that carry `canonical_event_json`. Use `dry_run: true` to count valid backfill, benchmark artifact invalidation, or vector-index candidates without writing rows or artifacts.

Retention legal holds are service-configured by policy ID, not by trace submitters. A trace envelope can suggest a retention policy, but only the authenticated service configuration decides whether that policy is under legal hold.

Tenant tokens can be configured as either `tenant_id:token` for contributor access or `tenant_id:role:token` where role is `contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`, `vector_worker`, `benchmark_worker`, `utility_worker`, or `process_eval_worker`. Add `;expires_at=<RFC3339>` or `;expires=<RFC3339>` to either syntax for a short-lived static-token bridge while production identity claims are still being built. The service can also accept signed HS256 tenant claims when `TRACE_COMMONS_SIGNED_TOKEN_SECRET` is set, and EdDSA/Ed25519 signed claims when an EdDSA public key is configured; these claims bind tenant id, actor/principal, role, allowed consent scopes/uses, and expiry in the bearer token instead of enumerating every token in service configuration. The claim allow-lists gate submission plus downstream exports, benchmark/ranker generation, process-evaluation labeling, and utility-credit jobs. Each static or signed token is treated as its own pseudonymous auth principal inside the tenant, and the principal hash excludes static-token expiry metadata. Reviewer/admin tokens can list tenant-local quarantine, approve or reject submissions, append delayed credit events, view tenant analytics, and export approved replay dataset slices. Review decisions and delayed credit mutations require non-empty operator reasons. Contributor tokens can submit, revoke, read their own token-principal credit/events, and sync status for their known submission ids, but cannot review, view tenant-wide analytics, append credit events, or export datasets.

Signed claim allow-lists also gate vector-index workers. A vector worker token
that carries only `aggregate_analytics` is rejected because vector payloads and
nearest-neighbor metadata are derived artifacts.

Production identity should replace static-token enumeration and HS256 bridge
claims with issuer-managed EdDSA/Ed25519 signed tenant upload claims. The
current EdDSA verifier supports default or `kid`-selected public keys, JSON or
file-backed EdDSA keysets with optional `not_before`/`not_after` activation
windows, issuer and audience checks, max-TTL policy, required JWT IDs, and
emergency `jti` denylists. The remaining issuer work is managed key
distribution and a production policy that accepts only the EdDSA issuer keyset.
Production claims must bind the tenant id, actor or job principal, role,
expiry, optional JWT ID, and allowed consent scopes/uses; the service still
derives the storage partition from the
authenticated claim after verification, not from envelope metadata supplied by
the contributor.

On submit, the service also writes a derived redacted-only record with:

- canonical summary and hash
- hash-based duplicate precheck
- placeholder novelty score for later vector replacement
- coverage tags for channel, tool, tool category, outcome, failure mode, and privacy risk
- aggregate analytics by status, privacy risk, task success, tool, tool category, and coverage tag

The current API remains intentionally file-backed under `TRACE_COMMONS_DATA_DIR` or `~/.ironclaw/trace_commons_ingest` for compatibility and easy local operation, with optional DB-backed read flags for contributor, reviewer metadata, replay/export selection, and audit surfaces. This branch also includes the first production storage bridge: optional DB dual-write metadata, optional encrypted local artifact storage, object-primary submit/review mode that avoids plaintext envelope body files while retaining file-backed metadata/audit compatibility records, and a fail-closed reconciliation gate for promotion jobs. PostgreSQL RLS policy migration V31 now covers the tenant-scoped Trace Commons metadata tables without `FORCE ROW LEVEL SECURITY`; production deployments still need transaction-local tenant context through every DB-backed runtime path before RLS can become the active trust boundary. Production deployments should finish promoting reviewer/export/analytics paths into DB/object-primary reads and move encrypted artifacts behind remote service-owned object storage before broad rollout.

## Production Hardening Roadmap

The current implementation is a usable MVP for local development and controlled internal pilots. A production Trace Commons deployment needs the following before broad tenant rollout:

### DB and Object Storage

- Promote the current dual-write mirror into relational metadata reads for all API surfaces and service-owned encrypted object storage for redacted trace bodies. Contributor credit/status, reviewer metadata, replay export selection, and audit reads already have opt-in DB-backed rollout gates.
- Keep metadata and object keys tenant-scoped from the auth-derived tenant id. Do not trust tenant fields in the envelope as storage partition keys.
- Store immutable submission records, append-only credit events, revocation tombstones, review decisions, export job manifests, and processing job state as separate records.
- Use row-level tenant policies or an equivalent authorization layer for every metadata query.
- Encrypt object storage at rest, require TLS in transit, and keep object bucket access behind service identities rather than reviewer/user tokens.
- Do not expose raw corpus bucket access or broad downloadable corpus snapshots. Individual trace reads, dataset builds, and training/evaluation exports should go through API-mediated, tenant-scoped jobs with per-source audit events and explicit purpose metadata.
- Version all derived artifacts. A redaction, vector, ranking, benchmark, or export worker must record input envelope hash, worker version, policy version, and output artifact id.

### Tenant RBAC and ABAC

- Move beyond static tenant tokens before production. Static tenant tokens can carry expiry attributes and the ingest service can accept HS256 signed tenant claims as interim bridges, with claim allow-lists already constraining submission and downstream export/worker use. The service can verify EdDSA/Ed25519 signed claims through default or `kid`-selected public keys and local/file keysets with activation windows; production should use only EdDSA/Ed25519 upload claims and still needs managed issuer distribution/governance before broad rollout.
- Enforce RBAC for contributor, reviewer, admin, trainer/export job, and service-worker roles.
- Add ABAC checks for consent scope, allowed use, privacy risk, review state, retention policy, revocation state, export purpose, and tenant data residency. Current tenant policy ABAC covers submission allowed scopes/uses plus replay, benchmark, and ranker export required uses, and signed-claim allow-lists should continue to constrain both submission and downstream worker/export consumption.
- Keep vector workers under the same ABAC model as export and utility workers: both the worker claim/policy and the source trace must permit a derived vector use, and aggregate-only traces must not enter derived vector indexes.
- Require privileged operations such as tombstone deletion to carry an explicit reason. Review decisions, delayed credit mutation, and destructive retention purges already require non-empty reasons or purposes, and export guardrails can require explicit replay, benchmark, and ranker export purposes.
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
- Accept only the safe projection: redacted text, allow-listed labels, counts, warnings, and summary metadata. Strip `text`, raw span strings, raw offsets, and unknown high-risk fields; unsupported span labels are mapped to `unknown` so malformed sidecars cannot smuggle emails, paths, or tokens through label names.
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
- periodic credit notice interval for CLI/web/runtime notifications

The runtime can call the same queue and flush behavior later after a task completes. When `flush-queue` runs under an enabled policy, it compacts duplicate queued contribution envelopes and orphan held sidecars, submits eligible traces autonomously, honors typed retry backoff for transient submission failures, and prints a credit update when the configured notice interval has elapsed.

`ironclaw traces queue-status` reports local autonomous queue readiness without exposing trace bodies: opt-in state, endpoint presence, bearer-token environment availability, capture toggles, selected-tool count, queued and held counts, typed retry/manual-review/policy hold counts, the next scheduled retry time, durable flush telemetry, retry/status-sync failure counters, last compaction reclaimed count, duplicate envelopes removed, orphan hold sidecars removed, sanitized held-reason counts, sanitized Endpoint/Credential/Network/NetworkOffline/NetworkDns/NetworkTimeout/NetworkConnectionRefused/HttpRejection/Policy/Queue/StatusSync/Submission/Unknown failure classes, and the same local credit summary used by `credit`. Diagnostics also expose aggregate warnings for schema version mismatch, consent policy mismatch, redaction pipeline mismatch, trace-card redaction pipeline mismatch, and malformed envelopes; those warnings include severity, production-promotion blocking flags, and safe recommended actions, but never include raw bodies or raw observed mismatch values. The authenticated web API exposes a narrower scoped queue status through `/api/traces/queue-status`, returning queued/held counts, durable telemetry, safe warning aggregates, and sanitized held queue entries for the current user only.

`ironclaw traces credit --notice` and `GET /api/traces/credit-notice` mark a due periodic credit notice without exposing central corpus rows. Notice summaries include pending/final totals, delayed ledger deltas, credit-event counts, and recent safe explanations when the local notice interval is due. Opted-in clients can acknowledge the current notice until the local credit fingerprint changes (`ironclaw traces credit --notice --ack` or `POST /api/traces/credit-notice` with `{"action":"acknowledge"}`), or snooze it for a bounded period (`--snooze-hours N` or `{"action":"snooze","snooze_hours":N}`). The local fingerprint is a hash over submission ids, lifecycle/status, credit totals, and credit-event metadata, not over explanation text or trace bodies.

The agent runtime also schedules an autonomous post-turn contribution pass after a response is persisted or a turn fails. It reads the authenticated user's scoped policy, verifies the thread still belongs to that user, captures the most recent turns from durable conversation history, locally redacts the envelope, queues it, and flushes eligible queued envelopes. If the flush produces a due credit notice, the agent sends a status update back through the originating channel. Independently, the long-running agent loop starts a periodic Trace Commons queue worker that scans the owner and active DB users, flushes opted-in scoped queues, honors typed retry backoff, and broadcasts any due credit notice it returns instead of silently consuming notice state.

During each queue flush and before web credit/submission responses, the client asks the ingestion API for status updates for locally known submitted ids. The status endpoint is tenant-bound by the bearer token and returns only records from that tenant's namespace, so delayed reviewer credit can be reflected locally without allowing broad corpus enumeration. Contributor credit, credit-event, and status-sync reads append safe aggregate read-audit rows with item counts only, not raw requested submission id lists. The authenticated web settings response also surfaces persisted held-queue reasons and the richer local credit report for the current user scope only; held queue responses contain submission ids and sanitized hold reasons, not queued envelope bodies.

In the authenticated web gateway, policy, queue, ledger, and revocation state are scoped under a hashed user/tenant directory rather than the global CLI policy. Envelopes carry a pseudonymous contributor id and a separate pseudonymous tenant scope reference, so the private ingestion service can attribute credit and enforce tenant boundaries without storing raw user ids in the trace body.

## Multitenant Permissioning

Trace contribution authorization must be derived from the authenticated request or runtime identity, not from fields inside a submitted envelope. Envelope fields such as `contributor.pseudonymous_contributor_id` and `contributor.tenant_scope_ref` are attribution/provenance metadata only.

For local capture:

- Web preview and autonomous runtime capture use the authenticated `user_id` as the trace scope.
- Conversation history is read through tenant ownership checks before a contribution envelope is built.
- Local policy, queue, submission history, revocation state, and credit records live under `trace_contributions/users/<hash>` for the authenticated user scope.
- The envelope includes a stable pseudonymous contributor id and a separate stable pseudonymous tenant scope reference. Neither includes the raw user id.

For the private ingestion service:

- The service should bind every request to a tenant from AuthN/AuthZ credentials, such as a tenant-scoped token, mTLS identity, or EdDSA/Ed25519 signed upload claim.
- It should reject requests where the authenticated tenant is not allowed to submit for the claimed tenant scope.
- Central metadata, credit ledger rows, revocation tombstones, privacy review queues, trace objects, and export jobs should all be keyed by the auth-derived tenant id plus the authenticated principal or contributor pseudonym. The auth-derived tenant id is the storage partition; envelope tenant references are never partition keys.
- RBAC/ABAC policies should allow contributors to see only their own submissions and credit, reviewers to see quarantined/redacted traces for permitted tenants, and trainer jobs to read approved slices through controlled jobs.
- Signed upload claims should carry allowed consent scopes and allowed uses so the same verified claim can limit submission, replay export, benchmark/ranker generation, process-evaluation labeling, and utility-credit jobs.
- Audit logs should record tenant id, actor id/job id, submission id, access reason, and export target for every individual trace read or mutation.
- The corpus should not be exposed as a raw bulk download. Researchers, trainers, and reviewers should access approved slices through scoped API routes or controlled jobs that write tenant-scoped manifests and read-audit rows.

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
- `GET /api/traces/credit-notice`
- `POST /api/traces/credit-notice`
- `GET /api/traces/queue-status`
- `GET /api/traces/submissions`
- `POST /api/traces/submissions/{submission_id}/revoke`

The web settings panel includes a Trace Commons tab for standing opt-in, autonomous submission controls, queue flushing, persisted held queue reasons, recent submissions, revocation, and richer credit/review totals. The chat composer also has a Trace button that previews the current thread's redacted envelope and can queue it for the same autonomous submission path. Local preview remains available without opt-in, but enqueue/manual-submit/autonomous acceptance now preflights the scoped standing policy and requires both opt-in and an ingestion endpoint before capture/redaction work is queued. Web enqueue and submit requests may not widen capture beyond the standing policy: if the policy disallows message text or tool payloads, crafted requests with those flags are rejected before a queue file is written.

## Implementation Status Matrix

| Area | Status | Maintainer notes |
|------|--------|------------------|
| Local opt-in policy and opt-out | Implemented MVP | CLI and scoped web/runtime policy files exist; submit token stays in environment. |
| Local preview, queue, flush, and credit display | Implemented MVP | CLI and web paths use local redacted envelopes and local submission metadata; `ironclaw traces queue-status` reports scoped policy readiness, bearer-token environment presence, queued/held counts, typed retry/manual-review/policy hold counts, next retry time, durable flush/status-sync telemetry, last compaction reclaimed count, duplicate-envelope and orphan-hold-sidecar removals, safe warning aggregates, sanitized held-reason counts, sanitized failure classes, and the local credit summary, while authenticated web activity exposes scoped queue/held counts and reloads persisted held queue reasons plus accepted/quarantined/rejected and delayed-credit report fields for the current user scope. Periodic credit notices can be acknowledged until credit changes or snoozed for a bounded number of hours through both CLI and authenticated web actions. |
| Deterministic local redaction | Implemented MVP | Includes generic secret/path scrubbing, stable placeholders, tool-aware payload handling, and Privacy Filter safe projection. |
| Privacy Filter sidecar integration | Implemented MVP | Local command/stdin/stdout path exists with safe output projection, non-fatal fallback, minimal child environment, stderr hashing, IO limits, and canary tests. Production container sandboxing and stricter output contracts remain. |
| Autonomous post-turn and periodic contribution | Implemented MVP | Runtime queues/flushed scoped envelopes after persisted or failed turns only when the scoped standing policy is enabled, has an ingestion endpoint, and the current redacted envelope is eligible for autonomous submission. Ineligible current traces are skipped instead of written as held queue files, while existing queue flushes and periodic credit-notice sync still run. A periodic agent-loop worker also flushes opted-in owner/active-user queues, stores retryable submission failures as typed redacted sidecars with capped exponential backoff, skips retry-held envelopes until due, compacts duplicate queued contribution envelopes and orphan held sidecars, classifies failures into sanitized local diagnostic buckets including offline/DNS/timeout/connection-refused network subtypes, and broadcasts returned credit notices. |
| Web Trace Commons settings and preview endpoints | Implemented MVP | Authenticated gateway endpoints and UI controls exist; server-side tenant/user checks remain the trust boundary, and queue/manual-submit paths preflight scoped opt-in policy before enqueueing. |
| Private ingestion service | Implemented MVP | Development/internal binary validates schema, reruns redaction, computes hashes/credit, enforces optional contributor-only tenant/principal hourly submission quotas for autonomous uploads, stores accepted/quarantined records, and serves review/status/export routes, including reviewer trace-list filtering by export/provenance purpose. It can now dark-launch DB dual-write metadata and encrypted envelope artifacts. |
| Tenant token roles | Partial | Static tenant tokens support contributor/reviewer/admin plus scoped export, retention, vector, and benchmark worker behavior; benchmark, retention, and vector workers now have dedicated `/v1/workers/benchmark-convert`, `/v1/workers/retention-maintenance`, and `/v1/workers/vector-index` routes as well as CLI helpers for those scoped routes and scoped compatibility routes where needed; optional `expires_at`/`expires` RFC3339 attributes reject stale bearer tokens before tenant attribution while preserving principal hashes over the token secret only; optional HS256 signed tenant claims bind tenant id, actor principal, role, issuer/audience when configured, allowed submission consent scopes/uses, expiry, optionally bounded `exp - iat` lifetime, and optional required `jti` claim without enumerating every bearer token; optional EdDSA/Ed25519 signed claims can be verified with default, `kid`-selected public keys, or local/file keysets with activation windows, and config status exposes safe total/EdDSA key counts without key material; optional tenant submission policies can restrict allowed consent scopes and trace-card uses at ingest and export time, DB-backed tenant policy records can drive submission and export policy reads behind `TRACE_COMMONS_DB_TENANT_POLICY_READS`, admin tokens can manage the current tenant's DB-backed contribution policy via `/v1/admin/tenant-policy` or the `ironclaw traces tenant-policy-get/set` CLI helpers, policy admin reads/writes are audited with safe hash-only policy metadata, and production-like deployments can require every submitting/exporting tenant to have a policy entry. Production upload claims should be EdDSA/Ed25519-only; the managed issuer keyset with activation windows, fuller central policy, RBAC/ABAC, and row-level tenant enforcement hardening remain. |
| Contributor credit ledger and delayed credit sync | Partial | Append-only local and central credit events exist, pending submission estimates are kept separate from settled final/ledger credit, reviewer/admin delayed credit mutation requires a reason and can resolve submission metadata through the DB reviewer read path when file metadata has been removed, benchmark conversion plus ranker candidate and pair exports append idempotent delayed utility credit with external artifact/job refs, DB credit events preserve training-vs-ranking utility types, terminal traces retain ledger events for audit but exclude those deltas from contributor totals, DB audit rows include typed credit-mutation metadata with hashed reason/reference fields, maintenance reconciliation reports file/DB ledger event ID gaps, and autonomous clients periodically notify opted-in users when submitted or later-revoked records receive ledger changes, including delayed ledger deltas and credit-event counts. Production needs anti-abuse review and stricter settlement policy. |
| Quarantine/review workflow | Partial | Reviewer/admin routes can list and decide on quarantined redacted traces; quarantine and active-learning queue items expose reviewer SLA/escalation metadata (`review_age_hours`, `review_escalation_state`, and `review_escalation_reasons`) plus optional DB-backed review lease assignment fields for prioritized triage; queue reads can filter lease state with `all`, `mine`, `available`, `active`, or `expired`; review decisions require a non-empty reason and cannot bypass another active reviewer lease; with DB reviewer reads enabled, reviewers/admins can claim or release durable tenant/principal-scoped leases, review decisions resolve submitted-envelope bodies through active DB object refs when available, can be configured to require active object refs, emit content-read audit rows, and mirror the reviewed envelope body as a fresh `review_snapshot` object ref without reclassifying the original submitted envelope; object-primary submit/review mode can also skip plaintext submitted/reviewed envelope body files. Production still needs richer assignment policy and retention/revocation gates. |
| Replay dataset export | Partial | Approved redacted slices can be exported by reviewer/admin tokens, production-like deployments can require explicit accepted/low-risk/consent-scoped export guardrails for replay, benchmark, and ranker-training exports plus caller-supplied export purposes and active DB object refs for body reads, tenant policy allowed-use ABAC gates replay/benchmark/ranker requests and source selection, DB metadata can drive replay export selection, benchmark/ranker exports can fail closed when selected sources lack active submitted-envelope object refs, submitted envelope bodies can resolve through active DB object refs for file or encrypted local artifact stores, object-primary replay export mode can require service-local DB object refs with no file fallback, manifests carry source-list hashes mirrored into audit `decision_inputs_hash`, replay exports mirror compact DB manifest rows and per-source item snapshots with source object refs plus invalidation timestamps, benchmark/ranker item rows link derived refs, active vector refs when indexed, and per-source object refs to file-backed or service-local encrypted benchmark/ranker artifacts, reviewer/admins can list replay manifest metadata, and each exported trace body read emits a tenant-scoped audit event. Production needs remote object storage, broader bulk export controls, and revocation propagation for already-published artifacts. |
| Analytics summary | Partial | Aggregate counts by status/risk/tool/coverage exist, including content-free process-evaluation aggregates for evaluated trace count, labels, rubric ratings, and score bands. Deployments can set a minimum cell-count threshold to suppress rare aggregate buckets before responses leave the service. Production still needs fuller privacy-budget accounting if exposed broadly. |
| Production relational DB and encrypted object storage | Partial | V25-V37 PostgreSQL schema plus libSQL schema slices, shared `TraceCorpusStore`, both backend implementations, optional ingest DB mirror with contributor, tenant policy, reviewer metadata, durable review lease fields, replay selection reads, policy-gated DB object-ref-backed replay envelope reads, vector-entry metadata, compact replay export manifest metadata, replay export item snapshots, durable retention job/item ledger rows, backend-enforced same-tenant/submission checks for derived object refs, vector-entry derived refs, and export item derived/object/vector refs, benchmark/ranker export artifact object refs, canonical audit payloads for DB verifier recomputation, DB-native audit append ordering, encrypted local artifact sidecar, service-owned local encrypted object-store mode, object-primary submit/review mode for plaintext-free envelope body storage, object-primary replay export object-ref read mode, maintenance-triggered DB mirror backfill for submissions plus existing file-side credit/audit/replay-manifest rows with isolated per-item failure reporting, derived presence/status/hash/invalid-source diagnostics, file/DB credit-ledger and canonical audit-event ID gap diagnostics, split export-manifest kind diagnostics, export item object-ref and invalid-source diagnostics, separate object-ref presence/readability/hash-mismatch diagnostics, vector index gap diagnostics, reader-projection parity diagnostics, and initial PostgreSQL tenant RLS policies exist. Remote object storage, parity enforcement, `FORCE RLS`/service role policy hardening, and broader object-primary reads remain. |
| Central audit log | Partial | File-backed audit rows now include optional hash-chain fields plus a maintenance verifier while preserving legacy JSONL compatibility, DB audit rows mirror those chain hashes and canonical hash payloads for file-backed events, and PostgreSQL/libSQL audit rows now carry tenant-scoped `audit_sequence` values assigned inside serialized append transactions that reject stale hash-chain predecessors. Maintenance emits a DB mirror report that checks hash-field format, previous-hash linkage, canonical-payload hash recomputation, DB action/metadata projection drift, and file/DB canonical audit-event ID gaps for mirrored rows. Audit routes and optional DB audit reads cover core submit/review/credit/revoke mutations, contributor credit/status reads with aggregate item counts, retention/purge artifact invalidations, tenant policy admin reads/writes, reviewer analytics/list/review-queue/audit-log reads, dataset/benchmark/ranker exports, per-trace replay export content reads, process-evaluation writes, and per-source derived-summary reads for benchmark conversion, ranker candidate/pair exports, and vector indexing. `object_ref_id` is mirrored when the body is read through a DB object ref; derived-summary read rows carry only safe surface/purpose/source identifiers. Privileged delayed credit, process-evaluation writes, review lease claim/release operations, and tenant policy mutations now mirror typed safe metadata with bounded values and hashes rather than raw request bodies. |
| Retention enforcement | Partial | Submit records persist retention policy ids and expiry timestamps; maintenance and the dedicated retention worker route mark expired submissions and derived records, mirror DB expiration/artifact/export-manifest invalidation with typed action-count audit rows and durable retention job/item ledger rows when configured, prune cached exports that reference expired sources, skip expiration/purge for operator-configured legal-hold retention policy IDs, and can explicitly purge expired file/encrypted local artifact copies by cutoff only when non-dry-run purge requests carry a non-empty purpose. Production still needs service-owned object storage deletion workflows. |
| Revocation propagation to derived artifacts | Partial | Current revocation marks local/file status, mirrors DB status, writes tenant-scoped first-writer-wins tombstones with redaction/canonical-summary hashes when available, authorizes DB-only revocation against the original contributor or reviewer/admin principal, rejects same-tenant re-ingest matching retained file-backed or DB-mirrored tombstone submission ids, redaction hashes, or canonical-summary hashes, invalidates DB-mirrored object refs, derived precheck rows, vector metadata entries, replay export manifest rows, replay export item rows, file-backed benchmark/ranker provenance manifests, and published benchmark artifacts by moving their registry state to revoked and evaluation state to inconclusive, blocks file-backed replay export, and applies the same DB invalidation path when maintenance discovers an existing file-backed revocation tombstone or already file-marked revoked submission. Production must invalidate external benchmark registries and worker caches. |
| Vector duplicate/novelty index | Partial | DB schema, storage contract, dedicated vector worker route, maintenance-triggered metadata indexer, object-ref/readability gating before non-dry-run vector writes, per-source vector-index content-read audits with source object refs, and reconciliation gap diagnostics now persist/vector-check vector-entry metadata, nearest trace ids, cluster id, duplicate score, novelty score, and invalidation state for accepted canonical summaries. Duplicate/novelty scoring now uses deterministic redacted-summary similarity with exact-hash matches as the strongest signal, and encrypted artifact storage can persist a redacted canonical-summary vector payload as a worker-intermediate object ref for later embedding-worker promotion. Real embedding generation, vector payload search, and model-backed duplicate/novelty workers are still future work. |
| Ranking/model utility pipeline | Partial | A trusted offline utility-credit worker route and CLI helper append idempotent delayed credit for accepted traces using `regression_catch`, `training_utility`, or `ranking_utility` plus an external job/artifact reference. Model-specific utility analysis and production settlement policy remain. |
| Benchmark conversion pipeline | Partial | Reviewer/admin conversion and the dedicated benchmark worker route can produce tenant-scoped benchmark candidate artifacts with schema-versioned registry/evaluation lifecycle metadata, consent/status/risk filters, source-list hashes, immediate DB source-status revalidation, audit events, per-source derived-summary read audits, derived artifact refs, durable provenance manifests that revocation/maintenance can invalidate, idempotent utility credit events, audited lifecycle updates for registry/evaluator state in file-backed and object-primary modes, and source-invalidation propagation that lifecycle-revokes published benchmark artifacts. The CLI now has `benchmark-lifecycle-update` for worker/reviewer automation against that lifecycle endpoint. Production still needs automated registry publication jobs, evaluator orchestration, and external benchmark registry invalidation. |
| Production sidecar operations | Partial | Sidecar launches now use timeout/IO limits, minimal environment inheritance, stderr hashing, non-fatal deterministic fallback, safe output projection with allow-listed span labels, and canary-secret regression coverage. Production still needs container sandboxing and deployment-specific sidecar isolation. |

## Credit

The client computes a local pending credit estimate from a trace value scorecard. The scorecard keeps privacy risk, quality, replayability, capped novelty, duplicate penalty, coverage, difficulty, dependability, and correction value as separate components before producing the online score.

Each local submission record can store append-only credit events. The initial event records the accepted submission estimate as pending credit; it is not treated as settled final credit unless a later review or utility process explicitly finalizes it. Benchmark conversion plus ranker candidate and pair exports currently append idempotent delayed utility events for accepted included sources. Trusted offline utility jobs can append regression, training, or ranking utility credit for accepted traces through the dedicated worker surface; reviewer value and abuse penalties stay on reviewer/admin mutation paths. Shapley-style or influence estimates can inform offline analysis, but should not be exposed as direct immediate payment logic.

The ingestion API can return a receipt with updated pending/final credit and explanations; those values are stored locally in `submissions.json`.

Delayed credit/status refresh uses:

```http
POST /v1/contributors/me/submission-status
Authorization: Bearer <tenant-token>

{ "submission_ids": ["..."] }
```

The response is an array of records visible to that authenticated principal. Missing ids are omitted, which keeps cross-tenant and same-tenant cross-principal probes indistinguishable from unknown submissions.

Status records keep pending submission estimates separate from settled credit. `credit_points_pending` remains the online estimate, `credit_points_final` is present only when explicit final settlement exists, and delayed ledger fields are included when review or downstream jobs have awarded later utility credit: `credit_points_ledger`, `credit_points_total`, and `delayed_credit_explanations`. When delayed ledger events exist, `credit_points_total` is computed as explicit final credit plus the delayed ledger delta, not as pending estimate plus ledger. If a trace is later revoked, expired, or purged, status sync reports a zero delayed ledger and a safe explanation that retained ledger events are excluded, while the contributor credit-events endpoint still hides those terminal-trace rows. Local autonomous clients store the total as the effective settled credit and reset the credit-notice timer, so opted-in users can be periodically informed about benchmark, regression, ranking/training, reviewer, or abuse-penalty adjustments without seeing other contributors' ledger rows. Periodic notice summaries include the delayed ledger delta and credit-event count alongside pending and final confirmed credit.

Reviewers/admins can append delayed credit after downstream utility is known:

```http
POST /v1/review/{submission_id}/credit-events
Authorization: Bearer <reviewer-token>

{
  "event_type": "benchmark_conversion",
  "credit_points_delta": 2.5,
  "reason": "Converted into replay benchmark run 2026-04-25",
  "external_ref": "benchmark-run:2026-04-25:trace-commons"
}
```

Trusted offline utility workers use a narrower bulk route for accepted traces:

```http
POST /v1/workers/utility-credit
Authorization: Bearer <utility-credit-worker-token>

{
  "event_type": "ranking_utility",
  "credit_points_delta": 1.25,
  "reason": "ranking eval utility",
  "external_ref": "ranker:nightly-42",
  "submission_ids": ["..."]
}
```

The CLI wrapper is:

```bash
ironclaw traces worker-utility-credit \
  --endpoint https://trace-ingest.internal \
  --bearer-token-env TRACE_COMMONS_UTILITY_CREDIT_WORKER_TOKEN \
  --event-type ranking-utility \
  --credit-points-delta 1.25 \
  --reason "ranking eval utility" \
  --external-ref ranker:nightly-42 \
  018f2b7b-0c11-72fd-95c4-1f9f98feac01
```

The worker route is intentionally limited to `regression_catch`, `training_utility`, and `ranking_utility`; it is not for `reviewer_bonus` or `abuse_penalty`.

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
