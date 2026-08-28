# Tenant BI Telemetry V0 — Replacement Foundation Plan

> **Status:** Ready for review. This plan supersedes the direct-SQL plan that
> previously occupied this path. It covers the replacement foundation PR only.

**Goal:** Collect tenant-local, privacy-safe telemetry through a non-blocking
injected recorder, aggregate frequent observations into UTC-hour records, and
persist them through the existing scoped filesystem plane without losing an
entire in-progress hour during ordinary deployments.

**Design:** [Tenant BI Telemetry V0 Design](../specs/2026-08-26-tenant-bi-telemetry-v0-design.md)
**Research:** [Tenant BI Telemetry V0 Shape Research](../../plans/2026-08-26-tenant-bi-telemetry-v0-research.md)

## Scope

This PR delivers one vertical production slice:

1. neutral typed observation and recorder contracts;
2. hourly aggregation and a typed scoped-filesystem repository;
3. one bounded asynchronous queue and lifecycle-owned worker;
4. a successful-and-failed terminal trigger settlement producer;
5. production composition over the existing mounted root filesystem; and
6. a real embedded-libSQL integration scenario proving trigger-to-durable-row
   behavior, restart readback, and tenant isolation.

The admin HTTP/download surface, model/run/lifecycle producers, daily rollups,
central analytics, cost estimation, dashboards, and purge/TTL are follow-ups.
The typed bounded read contract lands now because it is required for independent
durability verification and prevents the export PR from redesigning storage.

## Non-negotiable invariants

- Producers call one synchronous `try_record`; no producer awaits telemetry,
  opens storage, or observes persistence failure.
- Composition owns exactly one telemetry `ScopedFilesystem` handle, recorder
  queue, and worker. There is no global static singleton and no per-tenant
  filesystem-handle cache.
- The recorder receives the canonical trusted `ResourceScope`; there is no
  mirror tenant/user authorization DTO.
- Telemetry uses only scoped paths below `/tenant-shared/telemetry/v0`.
  It never receives raw `RootFilesystem` authority or constructs
  `/tenants/...` paths.
- Frequent facts become hourly aggregates. No run, inference, or tool-call row
  is durable telemetry.
- One drain may durably commit a prefix. Additive writes are never replayed
  after an ambiguous failure. This best-effort contract deliberately does not
  require a grouped atomic transaction.
- Every read is bounded by tenant scope, half-open UTC time range, page size,
  and maximum window. No offset pagination or unbounded scan is allowed.
- Heartbeats, prompts, responses, reasoning, raw errors, tool inputs/results,
  emails, display names, run/thread IDs, and cost estimates are excluded.
- Telemetry failure never changes trigger settlement or another product result.
- No telemetry deletion is introduced.

## Canonical runtime shape

```text
trusted producer
    │ try_record(ResourceScope, TelemetryObservation)
    ▼
Arc<dyn TelemetryRecorder>
    │ one bounded Tokio MPSC; try_send exactly once
    ▼
single lifecycle-owned worker
    │ aggregate by tenant/user/hour/family/dimensions
    ▼
FilesystemTelemetryRepository<F>
    │ ScopedPath + ResourceScope + bounded cas_update/query_ordered
    ▼
existing CompositeRootFilesystem
    ├── libSQL
    ├── PostgreSQL
    └── in-memory
```

Composition calls its existing `wrap_scoped` once. The resolver rebuilds the
tenant-aware `MountView` for each operation, so a single handle safely serves
observations from many tenants. The `/tenant-shared` alias maps to
`/tenants/{tenant}/shared`; telemetry never sees the physical tenant root.

## Contract changes

### Recorder

```rust
pub trait TelemetryRecorder: Send + Sync {
    fn try_record(
        &self,
        scope: ResourceScope,
        observation: TelemetryObservation,
    ) -> RecordOutcome;
}
```

Usage attribution comes from `scope.tenant_id` and `scope.user_id`.
System-sentinel usage is invalid. Lifecycle observations may name a bounded
subject user distinct from the authenticated actor. The queue owns the cloned
scope; no reference escapes the call.

### Repository

The domain owns a concrete typed `FilesystemTelemetryRepository<F>` holding
`Arc<ScopedFilesystem<F>>`; it does not add a backend-selection repository
trait. A private `TelemetryBatchSink` behavior port is the forced worker-test
seam and has exactly one production implementation. Public repository methods
accept a trusted `ResourceScope`, typed requests, and typed records only:

```rust
async fn ensure_indexes(&self, scope: &ResourceScope) -> Result<(), TelemetryStoreError>;
async fn apply_batch(
    &self,
    batch: ScopedTelemetryBatch,
) -> Result<BatchApplyReport, TelemetryStoreError>;
async fn read_activity_page(...) -> Result<TelemetryPage<HourlyUserActivity>, TelemetryStoreError>;
async fn read_model_page(...) -> Result<TelemetryPage<HourlyModelUsage>, TelemetryStoreError>;
async fn read_failure_page(...) -> Result<TelemetryPage<HourlyRunFailure>, TelemetryStoreError>;
async fn read_automation_page(...) -> Result<TelemetryPage<HourlyAutomationUsage>, TelemetryStoreError>;
async fn read_lifecycle_page(...) -> Result<TelemetryPage<LifecycleEvent>, TelemetryStoreError>;
async fn read_coverage_page(...) -> Result<TelemetryPage<CollectorCoverage>, TelemetryStoreError>;
```

`BatchApplyReport` reports the applied prefix and failed record count. The
worker does not retry an ambiguous additive record. Shared `cas_update`
handles concurrent writers and restart-safe increments.

The worker partitions every drain into tenant batches. Each batch retains one
trusted representative scope for mount resolution and records attribution from
every observation before aggregation; the repository rejects a record whose
projected tenant differs from the batch scope. `ensure_indexes` runs before a
tenant's first batch or read. A bounded process cache records successful
declarations, never holds a lock across I/O, and may safely re-declare an index
after eviction or restart.

## Durable layout and query contract

Relative scoped paths remain deterministic:

```text
/tenant-shared/telemetry/v0/hourly/activity/{hour}/{user}/{origin}.json
/tenant-shared/telemetry/v0/hourly/model/{hour}/{user}/{provider}/{model}.json
/tenant-shared/telemetry/v0/hourly/failure/{hour}/{user}/{category}.json
/tenant-shared/telemetry/v0/hourly/automation/{hour}/{user}/{kind}.json
/tenant-shared/telemetry/v0/lifecycle/{event_id}.json
/tenant-shared/telemetry/v0/coverage/{hour}/{collector_instance}.json
```

Each JSON `Entry` has a closed record kind, schema version, typed body, and
explicit indexed projection. Unknown versions and enum values fail closed.
Tenant identity is projected for readback validation but is not used to route
the scoped operation.

Ordered index shapes are expressed as equality prefix, ordered key,
tie-breaker:

| Read shape | Index keys |
|---|---|
| family by time | `tenant_id, record_family, window_start, tie_breaker` |
| provider by time | `tenant_id, record_family, provider_id, window_start, tie_breaker` |
| model by time | `tenant_id, record_family, effective_model_id, window_start, tie_breaker` |
| provider + model by time | `tenant_id, record_family, provider_id, effective_model_id, window_start, tie_breaker` |
| lifecycle by time | `tenant_id, record_family, occurred_at, event_id` |

The reader derives the leading `tenant_id` equality filter from
`scope.tenant_id`, then chooses the exact index for the supplied family and
dimension equality filters. Tenant remains leading even though the scoped path
also isolates the mount because ordered projections are physically shared.
Provider/model filters support all four combinations. It never passes
`Filter::Range` to `query_ordered`.

For an ascending half-open `[from, to)` request:

1. validate `from < to`, closed-hour policy, page size, and maximum range;
2. start after cursor `(from, minimum_tie_breaker)`, where the reserved minimum
   cannot be emitted by the tie-breaker encoder, which includes every real row exactly
   at `from`;
3. continue with the last returned opaque cursor;
4. stop before the first row whose ordered time is `>= to`; and
5. return at most the requested page size.

Contract tests must prove exact-`from` inclusion, exact-`to` exclusion,
multiple rows at each boundary, every provider/model filter combination,
cursor continuation, and a large pre-existing history that is not scanned
before `from`.

## Trigger ownership change

`ironclaw_triggers` extends `TriggerActiveRunState::Terminal` with a closed
`TriggerTerminalOutcome`, then extends its existing
`TriggerFireSettlementObserver` with `TriggerRunTerminalSettlement`. The
composition lookup maps process states without losing detail:

- `Completed` or `Stopped` → `Completed`;
- `Failed` or `Killed` → `Failed`;
- `Cancelled` → `Cancelled`; and
- `RecoveryRequired` → `RecoveryRequired`.

The trigger history status remains the existing `Ok`/`Error` projection used
by `clear_active_fire`; the new outcome is observational detail, not a schema
change. `active_cleanup` constructs the creator `ResourceScope` from the
trusted persisted `TriggerRecord` tenant/user/agent/project fields with a fresh
operation invocation ID. It never derives authority from display strings or
transport metadata. The terminal event includes that scope, trigger/fire/run
identities, trigger-owned automation kind, and terminal outcome.

`active_cleanup` emits it exactly once for both `Ok` and failed terminal
runs, only after trigger history and active-fire clearing are durable. Existing
accepted-submission and pre-submit-failure callbacks keep their meanings.
Composition adapts this event to `AutomationSettledObservation` and calls the
recorder without filesystem I/O.

## SQL replacement and compatibility

The direct-SQL telemetry work exists only on this unmerged feature branch and
never collected production data. This replacement PR must remove every part of
that abandoned path in the same transition:

- delete `ironclaw_telemetry/src/libsql.rs` and `postgres.rs`;
- remove `libsql`, `ironclaw_libsql_runtime`, `deadpool-postgres`, and
  `tokio-postgres` from the telemetry manifest;
- delete ADR 0005;
- remove telemetry from persistence-driver allowlists and private-driver tests;
- replace telemetry SQL dependency and same-layer inventory rows with the
  canonical filesystem dependency;
- update `.claude/rules/database.md`, domain guidance, telemetry README, and
  target-architecture documentation;
- update root `AGENTS.md`, the telemetry contracts README, and
  `docs/internal/reborn/contracts/triggers.md`;
- delete SQL schema assertions and dual-driver repository tests; and
- keep this file as the only executable telemetry implementation plan.

Developer databases may retain unreachable experimental SQL tables. They are
not migrated, read, or dropped. This is safe because no production composition
ever wrote telemetry. If evidence of a shipped producer or production rows is
found, implementation stops and this assumption is revisited before deletion.

Because this changes frozen storage placement and trigger settlement
semantics, Task 1 is a contract-change request and ratification gate. It
updates the storage-placement and trigger contracts before any production Rust
edit; Tasks 2–6 may begin only after those docs and their architecture/guidance
assertions agree.

Rollback after the replacement lands removes producer/composition wiring.
Filesystem records remain unread; rollback never deletes telemetry or canonical
LLM/product data.

## Metric availability proof

The foundation supplies the durable grammar and bounded reads needed by later
exports. Producer status is explicit so no metric is overstated.

| Requirement | Foundation status | Required durable facts |
|---|---|---|
| Tenant active in period | Available after trigger producer | any automation/activity count |
| Users whose automations ran/succeeded | Available | user, hour, automation kind, outcomes |
| Cron/once/manual mix | Available | automation kind and counts |
| DAU/WAU/MAU | Derivable after run producer | distinct user/hour activity |
| Activity events per WAU | Derivable after run producer | reported activity denominator + active users |
| Inferences per WAU | Derivable after model producer | inference count + active users |
| Provider/model adoption | Derivable after model producer | provider, model, user, inference count |
| Token usage | Derivable after model producer | reported input/output/cache counters |
| Failure rate/error mix | Derivable after run producer | outcomes + sanitized category |
| Average latency | Derivable after run producer | total latency + run count |
| User retention/churn | Derivable after run producer | weekly active user sets |
| Win-back | Derivable after run producer | weekly active sets and analyst-selected quiet window |
| Signups/member population | Diagnostic after lifecycle producer | stable lifecycle event IDs and member transitions |
| Activation/funnel | Diagnostic after run+lifecycle producers | member added, setup transitions, first successful activity |
| Users with configured automations | Diagnostic after lifecycle producer | routine create/enable/disable/delete |
| Tenant retention across companies | Unavailable | cross-tenant central analytics is out of scope |
| Revenue/estimated cost | Unavailable | deliberately excluded |
| P50/P95 latency | Unavailable | no histogram or individual latency rows |
| Heartbeat usage | Unavailable | deliberately excluded |

“Diagnostic” means best-effort lifecycle loss prevents an authoritative
population denominator. “Derivable” means analysts choose calendar/cohort
rules; IronClaw does not publish a blessed aggregate.

## Pseudo-query verification

These queries describe the logical exported views, not SQL storage. A test
fixture can load typed repository pages into equivalent in-memory relations or
CSV tables.

### Q1 — automation users and outcomes

```sql
SELECT automation_kind,
       COUNT(DISTINCT user_id) AS users,
       SUM(run_count) AS runs,
       SUM(completed_count) AS completed,
       SUM(failed_count) AS failed
FROM hourly_automation
WHERE window_start >= :from AND window_start < :to
GROUP BY automation_kind;
```

This proves unique users with automations that actually ran and the
cron/once/manual outcome mix.

### Q2 — active users and engagement

```sql
SELECT utc_week(window_start) AS week,
       COUNT(DISTINCT user_id) AS wau,
       SUM(run_count) AS runs,
       SUM(reported_activity_event_count) AS activity_events
FROM hourly_activity
WHERE window_start >= :from AND window_start < :to
GROUP BY utc_week(window_start);
```

`activity_events / wau` gives activity events per WAU. The same distinct-user
shape at day/month grain gives DAU/MAU and WAU/MAU.

### Q3 — model filters and inference per WAU

```sql
SELECT provider_id, effective_model_id,
       COUNT(DISTINCT user_id) AS users,
       SUM(inference_count) AS inferences,
       SUM(input_tokens) AS input_tokens,
       SUM(output_tokens) AS output_tokens,
       SUM(cache_read_input_tokens) AS cache_read_input_tokens,
       SUM(cache_creation_input_tokens) AS cache_creation_input_tokens
FROM hourly_model
WHERE window_start >= :from AND window_start < :to
  AND (:provider IS NULL OR provider_id = :provider)
  AND (:model IS NULL OR effective_model_id = :model)
GROUP BY provider_id, effective_model_id;
```

Divide total inferences by the WAU from Q2 for inference per WAU. Repository
conformance runs this fixture for no filter, provider only, model only, and both.

### Q4 — retention, churn, and win-back

```sql
WITH active AS (
  SELECT DISTINCT utc_week(window_start) AS week, user_id
  FROM hourly_activity
  WHERE window_start >= :from AND window_start < :to
)
SELECT previous.week,
       COUNT(*) AS prior_users,
       COUNT(current.user_id) AS retained_users
FROM active previous
LEFT JOIN active current
  ON current.user_id = previous.user_id
 AND current.week = previous.week + 1
GROUP BY previous.week;
```

Churn is `prior_users - retained_users`. Win-back selects users active this
week whose most recent earlier active week is at least analyst-selected `K`
weeks ago.

### Q5 — signup and activation diagnostics

```sql
WITH signup AS (
  SELECT subject_id AS user_id, MIN(occurred_at) AS added_at
  FROM lifecycle
  WHERE event_kind = 'member_added'
  GROUP BY subject_id
), first_ok AS (
  SELECT user_id, MIN(window_start) AS first_ok_at
  FROM hourly_activity
  WHERE completed_count > 0
  GROUP BY user_id
)
SELECT COUNT(*) AS observed_signups,
       SUM(CASE WHEN first_ok_at < added_at + INTERVAL '7 days'
                THEN 1 ELSE 0 END) AS observed_activated
FROM signup LEFT JOIN first_ok USING (user_id);
```

The manifest labels this diagnostic because either best-effort event may be
lost.

### Q6 — quality, latency, and coverage

```sql
SELECT SUM(failed_count) / NULLIF(SUM(run_count), 0) AS failure_rate,
       SUM(total_latency_ms) / NULLIF(SUM(run_count), 0) AS average_latency_ms,
       SUM(reported_tool_call_count)
         / NULLIF(SUM(tool_count_reported_run_count), 0)
         AS tool_calls_per_reported_run
FROM hourly_activity
WHERE window_start >= :from AND window_start < :to;
```

Failure-category rows provide the error mix. Coverage rows are grouped by hour
and collector instance; any reported drop, write failure, restart span, or open
hour marks the data partial. Coverage never claims that an outage successfully
recorded its own loss.

## Implementation tasks

Every task starts with the named failing test and ends with the narrowest green
checks. Structural deletion and behavioral additions remain separate commits.

### Task 1 — retire the SQL fork and pin the scoped boundary

**Tests first**

- Change architecture tests to reject telemetry driver dependencies and require
  `ironclaw_telemetry -> ironclaw_filesystem`.
- Add a guidance assertion that telemetry is absent from the SQL exception
  inventory and ADR 0005 is absent.
- Add contract assertions for tenant-leading ordered indexes and terminal
  success/failure settlement after durable active cleanup.

**Implementation**

- Delete the SQL adapters, SQL conformance suite, ADR 0005, dependencies, and
  allowlists listed in “SQL replacement and compatibility.”
- Ratify the contract change first by updating `storage-placement.md`,
  `triggers.md`, and `_contract-freeze-index.md` with scoped storage, exact
  outcome mapping, ordering, failure semantics, and caller-level acceptance
  tests. Only then change production code in later tasks.
- Update root `AGENTS.md`, the telemetry contracts/domain READMEs, domain
  guidance, database rule, and target architecture to name scoped filesystem
  persistence and terminal settlement accurately.
- Update the same-layer dependency inventory and crate budgets.

**Verify**

```bash
cargo test -p ironclaw_architecture_tests
python3 scripts/ci/docs_publication_boundary.py
python3 scripts/ci/check-target-tree.py
```

**Commit:** `refactor(telemetry): converge persistence on scoped filesystem`

### Task 2 — make scope part of the recorder contract

**Tests first**

Extend existing observation and buffered-recorder tests to prove:

- scope tenant/user is the only usage attribution source;
- system-sentinel usage is rejected;
- the scope is owned by the queued envelope;
- lifecycle subject user may differ without changing tenant authority;
- full/closed queue remains one non-blocking `try_send`; and
- observation validation stores no forbidden fields.

**Implementation**

- Change `TelemetryRecorder::try_record` to accept `ResourceScope`.
- Update typed observations to remove duplicated tenant/user usage authority.
- Preserve closed enums, identifier bounds, checked counters, and the existing
  no-op recorder for unwired callers.

**Verify**

```bash
cargo test -p ironclaw_telemetry_contracts
cargo test -p ironclaw_telemetry --test buffered_recorder_contract
cargo test -p ironclaw_telemetry --test hour_bucket_contract
```

**Commit:** `refactor(telemetry): carry trusted scope through intake`

### Task 3 — implement the scoped filesystem repository

**Tests first**

Replace SQL repository tests with one conformance suite over in-memory
`ScopedFilesystem`, then run the same contract against real
`LibSqlRootFilesystem`. Prove:

- deterministic relative paths and rejection of raw/caller paths;
- physical tenant isolation through two `ResourceScope` values;
- typed JSON/version/enum fail-closed decoding;
- same-key additive CAS, concurrent writers, retry exhaustion, and overflow;
- explicitly accepted prefix commit with no ambiguous replay;
- idempotent lifecycle events;
- all ordered indexes initialize idempotently per tenant, including duplicate
  initialization and cache eviction;
- exact `[from,to)` boundaries and cursor continuation;
- all provider/model filter combinations choose a compatible index; and
- an old history prefix is skipped by the starting cursor.

**Implementation**

- Add `FilesystemTelemetryRepository<F>` over
  `Arc<ScopedFilesystem<F>>`.
- Use `ScopedPath`, `Entry`, indexed projections, `ensure_index`,
  `query_ordered`, and shared bounded `cas_update`.
- Keep domain record grammar and path encoding private and typed.
- Delete `TelemetryRepository`, migration/admission helpers,
  `TelemetryScanRequest`, and `TelemetryScanPageRequest`. Retain and reuse
  `TelemetryPage<T>` plus the six typed record families behind explicit
  per-family read methods.
- Add only the private `TelemetryBatchSink` forced seam used by the worker and
  its fake; return `BatchApplyReport` and do not add a backend-selection trait.
- Declare only indexes consumed by foundation reads: tenant+family time,
  tenant+provider time, tenant+model time, tenant+provider+model time, and
  tenant+lifecycle time. Defer user-specific and subject-history indexes until
  their selectors land.

**Verify**

```bash
cargo test -p ironclaw_telemetry
cargo test -p ironclaw_filesystem
```

**Commit:** `feat(telemetry): persist hourly facts through scoped filesystem`

### Task 4 — preserve bounded worker and deployment flushing

**Tests first**

Extend the existing worker suite to prove:

- at most 512 observations per drain and at most one second wait;
- one consumer and no overlapping repository calls;
- aggregation by tenant/user/hour/dimensions;
- one failed drain does not stop later drains;
- coverage counts accepted, invalid, full, closed, and write-failed inputs;
- graceful close drains the queue within a five-second budget; and
- timeout aborts without blocking product shutdown indefinitely.

Use fake time and synchronization barriers, not sleeps.

**Implementation**

- Adapt the worker to scoped envelopes and `BatchApplyReport`.
- Retain one bounded Tokio MPSC queue and lifecycle handle.
- Keep diagnostics count-only and never log observation fields.

**Verify**

```bash
cargo test -p ironclaw_telemetry --test buffered_recorder_contract
```

**Commit:** `feat(telemetry): flush scoped aggregates asynchronously`

### Task 5 — add authoritative terminal trigger settlement

**Tests first**

Extend trigger worker tests through `active_cleanup` to prove:

- `Ok`, failed, cancelled, and recovery-required terminal results each emit one
  `TriggerRunTerminalSettlement`;
- history and active-fire clearing are durable before the callback;
- duplicate/continued cleanup does not emit twice;
- submitted-but-nonterminal and pre-submit failures emit none; and
- the event carries creator scope and trigger-owned automation kind.
- process lifecycle terminal variants preserve the exact mapping listed in
  “Trigger ownership change” through `TriggerActiveRunState`;
- the trusted scope is constructed from persisted trigger owner fields, not
  from run/display metadata.

Extend the existing composition observer tests to capture every recorder
argument and prove recorder loss never changes trigger settlement.

**Implementation**

- Add `TriggerTerminalOutcome` to `TriggerActiveRunState::Terminal`, map it in
  `ProcessActiveRunLookup`, and keep the existing history `Ok`/`Error` mapping
  solely for trigger persistence.
- Add the closed event and callback to the trigger-owned observer contract.
- Emit it from `active_cleanup` for every terminal outcome after settlement.
- Extend the composition observer to translate the event and call
  `try_record` exactly once.

**Verify**

```bash
cargo test -p ironclaw_triggers
cargo test -p ironclaw_composition trigger_poller
```

**Commit:** `feat(telemetry): observe terminal automation settlement`

### Task 6 — wire lifecycle and prove real libSQL durability

**Tests first**

Add one root scenario under `tests/integration/` and register it in the root
manifest and coverage map. The scenario must:

1. build production-profile composition over a temporary embedded libSQL DB;
2. create and fire a due trigger through the real poller;
3. drive its run to terminal `Ok`;
4. wait until active cleanup durably clears the fire and the terminal callback
   has executed;
5. close intake and await the telemetry worker drain;
6. drop the first runtime and reopen the same libSQL database;
7. create a fresh scoped telemetry repository over the reopened root;
8. read the exact automation aggregate through the original tenant scope; and
9. read the same relative request through another tenant scope and get zero.

The test must not call the recorder directly, query SQL, reach a private
adapter, or assert only that the turn completed.

**Implementation**

- Initialize indexes and start one worker after filesystem assembly.
- Inject only `Arc<dyn TelemetryRecorder>` into the trigger observer.
- Add a narrow lifecycle drain handle for shutdown and tests; do not expose a
  database handle.
- Update `tests/AGENTS.md` for the new scenario.

**Verify**

```bash
cargo test -p ironclaw_composition
cargo test -p ironclaw_integration_tests --test reborn_integration_tenant_telemetry
bash scripts/reborn-e2e-rust.sh
```

**Commit:** `feat(telemetry): wire scoped collection and libsql proof`

### Task 7 — PR-wide verification

```bash
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_telemetry_contracts
cargo test -p ironclaw_telemetry
cargo test -p ironclaw_filesystem
cargo test -p ironclaw_triggers
cargo test -p ironclaw_composition
cargo test -p ironclaw_architecture_tests
cargo test -p ironclaw_integration_tests --test reborn_integration_tenant_telemetry
bash scripts/ci/check-composition-budget.sh
python3 scripts/ci/docs_publication_boundary.py
python3 scripts/ci/check-target-tree.py
```

Inspect changed production files for `.unwrap()`, `.expect()`, raw tenant
paths, SQL/driver names, unbounded pages, forbidden telemetry fields, and lost
error causes. The PR body must state:

- compatibility: no shipped SQL telemetry data exists;
- rollback: remove wiring and leave scoped records unread;
- failure semantics: best-effort, bounded, and partial-prefix writes;
- verification evidence per test tier; and
- follow-ups: admin export, remaining producers, complete metric fixture, and
  later rollups.

## Follow-up sequence

1. Run/model/lifecycle producers and the complete metric fixture.
2. Authenticated tenant-admin bounded export over the same read contract.
3. Optional daily rollups only after measured export/query pressure warrants
   them.
4. Central cross-tenant analytics only with a separately approved hosted data
   plane and deployment/privacy contract.

## Stop conditions

Stop implementation and revise the design if any of these become true:

- production SQL telemetry rows or a shipped reader are discovered;
- scoped filesystem cannot start at `from` without scanning prior history;
- terminal success cannot be observed after durable trigger settlement without
  special-casing the shared poller;
- a required metric needs forbidden content or per-run durable rows; or
- deployment shutdown tests show routine loss materially larger than the
  bounded queue tail.
