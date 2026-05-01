# Reborn Events And Projections Contract

Reborn runtime events and audit records have two distinct layers:

- `ironclaw_events` owns the redacted record vocabulary, cursor types, sink traits, and durable log traits.
- `ironclaw_reborn_event_store` owns standalone Reborn backend selection and storage adapters for those traits.

The current standalone durable backends are JSONL, PostgreSQL, and libSQL. Each stores runtime and audit streams separately, keyed by `(stream_kind, tenant_id, user_id, agent_id)`, and persists cursor envelopes so a process restart can replay from the last seen cursor.

## Profile Rules

- `LocalDev` and `Test` may explicitly use in-memory stores.
- `Production` rejects in-memory stores before returning a service graph.
- `Production` may use JSONL only when the config explicitly accepts single-node durable storage.
- PostgreSQL and libSQL adapters are available behind the crate's `postgres` and `libsql` features. Their schema files live in `crates/ironclaw_reborn_event_store/migrations/`, and the factory runs those migrations before returning the service graph.
- If the crate is compiled without a requested SQL backend feature, the factory fails closed with a redacted backend-unavailable error.

## Replay Semantics

Durable backends must match `InMemoryDurableEventLog` cursor behavior:

- cursors are monotonic per `(stream_kind, tenant, user, agent)`;
- `read_after_cursor(None)` starts at origin;
- `limit == 0` is rejected;
- cursors beyond the stream head return `ReplayGap`;
- retained-history gaps return `ReplayGap` rather than silent loss;
- `ReadScope` filtering is enforced by the backend;
- records filtered out by `ReadScope` still advance the scanned cursor.

## No-Exposure Rules

Durable rows and JSONL files must not add raw secrets, host paths, request payloads, runtime output, approval reasons, or backend detail strings. Connection and migration failures are reported through redacted backend/operation errors. Event constructors and serialization enforce runtime `error_kind` sanitization; producer crates remain responsible for constructing metadata-only audit envelopes.

## Projection Boundary

Product-facing timeline, status, approval, auth, tool-call, process, resource, and memory views should be projections over durable logs, not a second source of truth. Projection services must tolerate replay gaps with explicit snapshot/rebase behavior and must not mutate control-plane state while deriving read models.
