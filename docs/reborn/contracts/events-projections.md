# Reborn Events And Projections Contract

Reborn runtime events and audit records have two distinct layers:

- `ironclaw_events` owns the redacted record vocabulary, cursor types, sink traits, durable log traits, and in-memory reference logs.
- `ironclaw_event_projections` owns product-facing projection DTOs and replay-derived read models over those durable log traits.

Concrete JSONL, PostgreSQL, and libSQL durable backends are storage adapters for `ironclaw_events` traits. They are intentionally not part of the projection API.

## Projection Mode

The first projection slice is on-demand replay:

```text
DurableEventLog
  -> ReplayEventProjectionService
      -> ThreadTimeline
      -> RunStatusProjection
```

This proves the read-model boundary without adding a materialized projection repository or new database schema. A future materialized store can checkpoint the same `ProjectionCursor` shape and feed the same DTOs if replay cost becomes too high.

Deferred option: this PR does not add a `ProjectionStore` or persistent projection rows. If one is introduced later, it must remain behind a trait and preserve PostgreSQL/libSQL parity.

## Projection API Shape

`EventProjectionService` exposes:

- `snapshot(ProjectionRequest) -> ProjectionSnapshot`
- `updates(ProjectionRequest) -> ProjectionReplay`

`ProjectionRequest` carries an explicit `ProjectionScope`, optional `ProjectionCursor`, and bounded `limit`. `ProjectionScope` is built from a caller-authorized `ResourceScope` into:

- an `EventStreamKey` for `(tenant, user, agent)`;
- a tightened `ReadScope` for project, mission, thread, and optional process filtering.

Product adapters should request these projections instead of reading durable event/audit tables or backend files directly.

## Cursor And Gap Semantics

Projection cursors wrap durable event cursors so consumers do not treat raw durable cursor internals as a product API. Replay gaps, stale cursors, or cursors from the wrong stream map to `ProjectionError::RebaseRequired` with cursor metadata. Callers must request a fresh scoped snapshot/rebase instead of silently continuing after missing facts.

Projection source failures are observable as projection errors, but the projection service does not mutate the durable event log or any kernel source-of-truth state.

## Initial Coverage

The first slice projects runtime events into:

- `ThreadTimeline`: ordered metadata-only timeline entries for dispatch and process lifecycle events.
- `RunStatusProjection`: per-invocation status derived from the latest visible runtime/process event.

Run statuses are intentionally projection-local:

- dispatch requested, runtime selected, or process started -> `running`
- dispatch succeeded or process completed -> `completed`
- dispatch failed or process failed -> `failed`
- process killed -> `killed`

For spawned/background processes, `dispatch_succeeded` is treated as an acknowledgement when a process is already active. The run remains `running` until a process terminal event is replayed.

Approval/auth gates, resource/cost, memory activity, and mission progress are deferred until their source event coverage is stable. In particular, approval resolution audit exists today, but a complete `ApprovalGate` projection also needs a reliable approval-request source event or store integration.

## No-Exposure Rules

Projection DTOs must not expose raw input, raw output, secrets, raw host paths, approval reasons, invocation fingerprints, backend detail strings, or provider/runtime error payloads. They may expose stable metadata such as capability id, runtime kind, provider id, process id, output byte counts, sanitized error kind, and timestamps.

Tests must include cross-scope non-leak coverage and sentinel checks for projection output and projection error strings.
