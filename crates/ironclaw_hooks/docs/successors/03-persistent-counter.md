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
      event_id      uuid PRIMARY KEY -- for replay dedup
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
      event_id      uuid PRIMARY KEY
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
