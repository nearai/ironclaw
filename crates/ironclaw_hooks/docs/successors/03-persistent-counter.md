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
3. **Replay refusal**: re-emitting a recorded invocation timestamp
   must NOT double-count. The backend stores `(timestamp, run_id,
   event_id)` so duplicate-event detection works at replay time.
4. **Backend-agnostic**: the predicate evaluator depends on the trait,
   not a specific store.

## Likely surface

```rust
#[async_trait]
pub trait PredicateStateBackend: Send + Sync {
    async fn record_invocation(
        &self,
        key: &InvocationKey,           // (tenant, hook_id, capability)
        timestamp: SystemTime,
        event_id: RuntimeEventId,
    ) -> Result<(), PredicateBackendError>;

    async fn count_in_window(
        &self,
        key: &InvocationKey,
        window: Duration,
    ) -> Result<u32, PredicateBackendError>;

    async fn record_value(
        &self,
        key: &ValueKey,                // (tenant, hook_id, capability, field)
        timestamp: SystemTime,
        value: Decimal,
        event_id: RuntimeEventId,
    ) -> Result<(), PredicateBackendError>;

    async fn sum_in_window(
        &self,
        key: &ValueKey,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError>;

    async fn evict_older_than(&self, cutoff: SystemTime) -> Result<u64, PredicateBackendError>;
}
```

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
  dual-backend story (per `src/db/CLAUDE.md`).

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
  Plan: batched writes via the dispatcher's tick boundary, with
  reads cached for the current dispatch.

## Effort

Medium-Large. Schema migration + backend impls are mechanical; the
performance-conscious read/write batching is the design-discussion
piece.
