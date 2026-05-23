# Successor PR: persistent predicate counter

> Successor work from PR #3573. Current sliding-window counter state is
> in-memory only and resets on every restart. This PR adds a durable
> backend so rate-limit / value-cap predicates survive process
> lifecycles.

## Scope

Add a `PredicateStateBackend` trait + Postgres / libSQL impls so
`PredicateEvaluator` can persist its counter state. Threat-model D5
(LRU eviction at 8192 keys per map) still applies in-memory; the
durable backend is the source of truth.

## Required behavior

1. **Cross-process consistency**: two host processes against the same
   tenant share counter state. Run-1 increments → run-2 sees the
   updated count.
2. **Restart survival**: counter state survives process restart.
3. **Replay refusal**: re-emitting a recorded invocation must NOT
   double-count. The backend dedupes on `event_id` (a sanitized
   `RuntimeEventId` hex) so duplicate detection works across restart
   / replay.
4. **Backend-agnostic**: the predicate evaluator depends on the trait,
   not a specific store.

## Likely surface

```rust
#[async_trait]
pub trait PredicateStateBackend: Send + Sync {
    /// Atomic record-and-read: the implementation MUST perform the
    /// insert AND the in-window count read under a single
    /// lock/transaction. Splitting them is a race that lets the cap
    /// drift past `max` (codex Critical from #3635).
    async fn record_invocation(
        &self,
        key: &InvocationKey,           // (tenant, hook_id, capability)
        timestamp: DateTime<Utc>,      // project-wide convention; see src/db/mod.rs
        event_id: &PredicateEventId,   // opaque hex; durable backends UNIQUE-constraint on it
        window: Duration,
    ) -> Result<u32, PredicateBackendError>;

    async fn record_value(
        &self,
        key: &ValueKey,                // (tenant, hook_id, capability, field)
        timestamp: DateTime<Utc>,
        event_id: &PredicateEventId,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError>;

    /// Garbage-collect rows older than `cutoff`. Operator runs this as
    /// a reaper task, typically at the slowest configured window.
    async fn evict_older_than(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, PredicateBackendError>;
}
```

**Clock note (responding to gemini's review on the prior draft):** the
in-memory backend that already shipped in PR #3635 uses `Instant`
because it's process-local and monotonic. Durable backends serialize
across processes, so they must use `chrono::DateTime<Utc>` to match
the rest of the project (`src/db/mod.rs`, `ironclaw_events`). The
trait will accept `DateTime<Utc>` and the existing in-memory backend
will gain a thin shim mapping `Instant`-driven callers to a fixed
reference point.

**Run scope:** the trait does NOT carry `run_id` directly. The
sliding-window state is per-tenant + per-hook-id; replay refusal is
driven by `event_id` (which is `RuntimeEventId` from `ironclaw_events`,
itself already keyed to the current run's emission). An earlier draft
mentioned storing `run_id` alongside; that's redundant given the event
id's uniqueness contract and was removed in this revision.

## Backends

- **`InMemoryPredicateStateBackend`**: keeps current behavior; used
  for tests + the standalone `ironclaw_hooks` integration tests that
  don't want to spin up a database.
- **`PostgresPredicateStateBackend`**: production. Schema:
  ```sql
  CREATE TABLE hook_invocation_events (
      tenant_id     text NOT NULL,
      hook_id       bytea NOT NULL,
      capability    text NOT NULL,
      occurred_at   timestamptz NOT NULL,
      event_id      uuid NOT NULL,
      -- Dedup is scoped to the counter key, NOT global on `event_id`.
      -- `caller_event_id` is per capability invocation, so two
      -- predicate-backed hooks observing the same invocation share the
      -- same event_id; a global PRIMARY KEY on event_id would let the
      -- first hook's INSERT win and silently undercount the second
      -- hook's bucket (serrrfirat HIGH on PR #3635 5-15 review). Same
      -- scope as the trait's replay-refusal contract: dedup within
      -- (tenant_id, hook_id, capability) only.
      PRIMARY KEY (tenant_id, hook_id, capability, event_id)
  );
  CREATE INDEX ix_hook_invocation_events_window ON hook_invocation_events
      (tenant_id, hook_id, capability, occurred_at DESC);

  CREATE TABLE hook_value_events (
      tenant_id     text NOT NULL,
      hook_id       bytea NOT NULL,
      capability    text NOT NULL,
      field         text NOT NULL,
      occurred_at   timestamptz NOT NULL,
      value         numeric NOT NULL,
      event_id      uuid NOT NULL,
      -- Same per-counter-key dedup scope as the invocation table; the
      -- additional `field` dimension is part of the counter key.
      PRIMARY KEY (tenant_id, hook_id, capability, field, event_id)
  );
  CREATE INDEX ix_hook_value_events_window ON hook_value_events
      (tenant_id, hook_id, capability, field, occurred_at DESC);
  ```
- **`LibSqlPredicateStateBackend`**: parity with the rest of the
  dual-backend story (per `src/db/CLAUDE.md`). Same shape as the
  Postgres impl, with two material differences mandated by
  `src/db/CLAUDE.md`:
  - `value` column is `TEXT NOT NULL` (not numeric), because libSQL's
    integer/real types can't preserve `rust_decimal` precision; the
    backend serializes via `Decimal::to_string()` / `from_str()` at
    the row boundary.
  - `occurred_at` is stored as ISO-8601 `TEXT` (libSQL convention).

## Migration / coexistence

- `PredicateEvaluator::new()` keeps the in-memory default.
- `PredicateEvaluator::with_backend(backend)` opt-in for production.
- The registrar / Reborn factory wires the backend per host.

## Required tests

1. **Replay refusal**: record the same `event_id` twice → second call
   no-ops, count unchanged.
2. **Cross-process**: two backends pointing at the same database,
   one increments, the other reads the updated count.
3. **Restart survival**: backend roundtrip after a connection cycle.
4. **Eviction policy**: `evict_older_than` clears expired rows; called
   periodically by a reaper task.
5. **Tenant isolation**: tenant A's writes don't show up in tenant B's
   reads (already pinned for in-memory by `HistoryKey` — confirm
   backend-side).

## Risk

- Touches `src/db/` migrations machinery. See `.claude/rules/database.md`
  for the dual-backend conventions.
- Performance: every predicate evaluation becomes a DB round-trip.
  An earlier draft suggested batching writes at the dispatcher's tick
  boundary. **That conflicts with requirement #1 (cross-process
  consistency)**: a deferred write from host A wouldn't be visible to
  host B's read until the next tick, so two concurrent hosts could
  each see "under cap" simultaneously and both proceed past `max`
  (gemini's review on the prior draft). Resolution: the v1 production
  backend keeps writes synchronous (read-your-own-writes within the
  call); the in-process cache stays per-dispatch-only. A future
  optimization could batch *reads* — collapse N predicate evaluations
  in one dispatch into a single batch SELECT — but never writes.

## Effort

Medium-Large. Schema migration + backend impls are mechanical; the
performance-conscious read/write batching is the design-discussion
piece.

---

## Landed shape (durable-backend split PRs 1–4, final)

The plan above shipped across a four-PR split. This section records the
**final landed contract** — it is the source of truth where it differs
from the speculative "Likely surface" above.

### Public async trait

`ironclaw_hooks::predicate_state::PredicateStateBackend` is `pub`,
`#[async_trait]`, `Send + Sync`. The argument order settled differently
from the draft (`now` precedes `event_id` is NOT how it landed):

```rust
#[async_trait]
pub trait PredicateStateBackend: Send + Sync {
    async fn record_invocation(
        &self,
        key: &InvocationKey,           // (hook_id, tenant_id, capability)
        event_id: &PredicateEventId,   // host-assigned; dedup is per-key, not global
        now: DateTime<Utc>,            // caller-supplied clock basis (see below)
        window: Duration,
    ) -> Result<u32, PredicateBackendError>;

    async fn record_value(
        &self,
        key: &ValueKey,                // InvocationKey + field
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError>;

    fn evictions_observed(&self) -> u64;

    async fn evict_older_than(&self, cutoff: DateTime<Utc>)
        -> Result<u64, PredicateBackendError>;
}
```

The whole surface is `DateTime<Utc>` (no `Instant` shim survived) — the
in-memory backend was rewritten to take a caller-supplied wall clock so
all three backends share one clock basis.

### Caller-supplied clock basis (clock-skew semantics)

All three backends trim the sliding window against the **caller-supplied
`now`**, NOT a server clock or the earliest stored entry. The window
cutoff for a call is `now - window` (saturating `chrono` arithmetic), and
the trim is strict `<` so an entry whose timestamp equals the cutoff is
**retained**. A host whose clock is skewed ahead therefore trims earlier
entries via *its own* `now`. This is verified in the parity matrix's
clock-skew scenario (PR 4/4): two hosts passing different `DateTime<Utc>`
against the same key behave deterministically per the clock each call was
given. The host runtime is responsible for supplying a sane `now`
(`Utc::now()` in production).

### Three backends

| Backend | Crate | Durable? | Multi-host dedup? |
|---------|-------|----------|-------------------|
| `InMemoryPredicateStateBackend` | `ironclaw_hooks` | no (process-local) | **no** — dedup is process-local |
| `PostgresPredicateStateBackend` | `ironclaw_hooks_postgres` | yes | yes (SQL `UNIQUE`/`PRIMARY KEY`) |
| `LibSqlPredicateStateBackend` | `ironclaw_hooks_libsql` | yes | yes (SQL `UNIQUE`/`PRIMARY KEY`) |

Dedup is scoped to the counter **key** — `(tenant_id, hook_id,
capability[, field], event_id)` — not global on `event_id`, so two
predicate-backed hooks observing the same capability invocation (sharing
a `caller_event_id`) do not undercount each other.

### Fail-closed cap semantics

The per-key sliding window has a sample cap, `MAX_SAMPLES_PER_KEY`
(4 096). Filling a key to the cap with distinct in-window ids succeeds;
the next **distinct** in-window id returns
`PredicateBackendError::WindowOverflow` — **fail closed**, never a silent
oldest-sample eviction (that would weaken cap enforcement and break replay
refusal). A **replay** of an already-recorded in-window id at the cap is
still a dedup no-op (returns the unchanged count), so replay refusal
survives the cap boundary. The evaluator maps `WindowOverflow` to the
restrictive `on_exceeded` action (DENY / PauseApproval), never a silent
Allow. This is the uniform contract across all three backends
(`record_invocation_overflow_is_fail_closed`, #3929) and is cross-asserted
identical in the parity matrix.

### LRU eviction — intended divergence

- All three backends enforce a **per-tenant / per-scope** LRU quota,
  `MAX_KEYS_PER_TENANT` (2 048): a noisy tenant flooding distinct scopes
  evicts *its own* oldest scopes (LRU victim = least-recently-active key)
  and `evictions_observed()` advances; a quiet co-tenant's scope is never
  evicted. This dimension is cross-asserted identical (same victim, same
  eviction count) in the parity matrix's `lru` script.
- The in-memory backend **additionally** enforces a global
  `MAX_HISTORY_KEYS` (8 192) cap across all tenants, because a process has
  a bounded heap. The durable backends do **not** have a global key
  ceiling (a database is the source of truth and is reaped by
  `evict_older_than`, not by a fixed key count). This is an **intended**
  divergence; the parity matrix deliberately stays under the per-tenant
  quota so it compares apples-to-apples and does not exercise the global
  cap.

### Multi-host guarantees (proven in PR 4/4)

The cross-backend adversarial parity suite (`ironclaw_hooks_parity`)
proves the durable backends provide, and the in-memory backend explicitly
does not:

1. **N concurrent writers across 2+ hosts** against one database — no
   count/sum desync, exactly-once counting (`BEGIN IMMEDIATE` / row-lock
   serializes the read-modify-write).
2. **Cross-host replay** — an id recorded on host A is a dedup no-op when
   replayed on host B against the same key (the SQL uniqueness constraint
   enforces dedup across every host pointing at the same DB).
3. **LRU eviction race** — concurrent inserts past `MAX_KEYS_PER_TENANT`
   hold the quota deterministically; `evictions_observed()` advances.
4. **Per-key cap under attacker flood** — fail-closed `WindowOverflow`,
   bounded; the count never exceeds the cap.
5. **Clock-skew** — see the clock-basis section above.

### A3 deferral — CLOSED

Threat-model finding **A3 (multi-host replay bypass)** from #3635 — that
the in-memory backend's process-local dedup lets a replayed `event_id`
double-count against the logical cap when two hosts share a tenant — is
**closed** by the durable backends and verified by PR 4/4's
`cross_host_replay_exactly_once` parity scenario. The SQL
`PRIMARY KEY (tenant_id, hook_id, capability[, field], event_id)`
constraint makes a cross-host replay a no-op `ON CONFLICT DO NOTHING`, so
exactly-once counting holds across every host pointing at the same
database. The in-memory backend's process-local limitation remains
documented on `PredicateStateBackend` (it is correct for single-process
deployments); production multi-host deployments MUST use a durable
backend, which is now the source of truth.

### Test layout

- **Trait + in-memory + contract harness** (`ironclaw_hooks`, PR 1/4):
  the `predicate_state::contract` module + `predicate_backend_contract_test!`
  macro behind the `contract-tests` feature.
- **Postgres backend + contract + adversarial** (`ironclaw_hooks_postgres`,
  PR 2/4): env-gated on `IRONCLAW_HOOKS_POSTGRES_URL` / `DATABASE_URL`.
- **libSQL backend + contract + adversarial** (`ironclaw_hooks_libsql`,
  PR 3/4): embedded temp-file db, runs anywhere.
- **Cross-backend parity matrix + multi-host adversarial**
  (`ironclaw_hooks_parity`, PR 4/4): one scripted sequence fed to all
  three backends with cross-assertion of identical observation logs;
  multi-host scenarios behind `--features integration`. The in-memory and
  libSQL legs run unconditionally; the Postgres leg compiles under
  `--features postgres` and runs only with a reachable DB URL (a
  real-Postgres CI run is required to fully exercise the Postgres parity
  leg before merge).
