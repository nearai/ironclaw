# IronClaw Reborn events and projections contract

**Date:** 2026-04-25
**Status:** Draft contract
**Depends on:** `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

Realtime progress, durable transcript, audit history, and UI projections are different products. They can share event records, but they must not be collapsed into one owner.

This contract defines the boundary between:

- realtime event delivery
- durable audit/history
- transcript milestones
- derived read models/projections
- transport-specific streams

---

## 2. Event layers

| Layer | Purpose | Source of truth? |
| --- | --- | --- |
| Realtime event bus | UI progress, live logs, orchestration, reconnect tail | No |
| Durable audit/history | replay, accountability, debugging, compliance, learning | Yes for audited facts |
| Transcript/thread history | user-visible conversation and durable thread milestones | Yes for conversation history |
| Projection/read model | sidebar, activity, job, project, harness, progress views | No; rebuildable |
| Transport stream | SSE/WebSocket/channel-specific delivery | No |

Rules:

- losing a realtime connection must not corrupt transcript or audit state
- projections must be rebuildable from durable state/events
- transport adapters may cache delivery cursors but do not own business state

The durable append log plus scoped replay cursor envelope is the substrate. It must be usable by implementation agents and caller-level tests before product transports are complete. SSE/WebSocket delivery and UI-specific projections are downstream integrations over that substrate, not prerequisites for landing the substrate.

Reborn runtime events and audit records have two owning crates:

- `ironclaw_events` owns the redacted record vocabulary, cursor types, sink traits, and durable log traits.
- `ironclaw_reborn_event_store` owns standalone Reborn backend selection and storage adapters for those traits.

---

## 3. Event identity and ordering

Every event emitted by runtime services should carry:

- event id
- event type
- timestamp
- correlation id
- relevant scope ids
- optional thread id
- optional run id
- optional invocation id
- redacted payload

Ordering guarantees should be explicit per stream:

- per-thread ordering for thread/run events
- per-run ordering for run progress
- global ordering only if a durable event store provides it

Do not require global ordering for all V1 events unless implementation pressure demands it.

---

## 4. Event classes

Minimum vocabulary classes:

| Class | Examples |
| --- | --- |
| Runtime events | process started/stopped/output, WASM invocation started/completed, sandbox event |
| Run-state events | run started, blocked, resumed, completed, failed, cancelled |
| Domain events | thread step added, mission created, job progress, subagent completed |
| Audit events | approval requested/resolved, secret accessed, network request made, budget denied |
| Extension lifecycle events | installed, activated, disabled, upgraded, capability surface changed |
| Projection events | read model invalidated, projection rebuilt, snapshot emitted |

Audit events are not simply realtime events with a longer retention period. They have stricter redaction and integrity requirements.

---

## 5. Projection reducer contract

A `ProjectionReducer` consumes durable state and selected events to produce read models.

Examples:

- conversation sidebar
- active run progress
- job list
- project/thread visibility
- extension capability surface
- approval/auth pending gates
- harness/check status

Reducer rules:

- deterministic for the same input state/events
- side-effect free
- rebuildable after restart
- may cache output, but cache is not source of truth
- must tolerate unknown future event types by ignoring or preserving them according to version policy

---

## 6. Reconnect and resume

Reconnect flow:

```text
client reconnects with last_event_id
-> EventStreamManager validates stream scope
-> replay available events after last_event_id
-> ProjectionReducer supplies current snapshot if replay gap exists
-> transport resumes live tail
```

`EventStreamManager` is the transport-agnostic facade over domain projection
services. It routes scoped runtime and audit replay requests through their
owning projection services and preserves their domain-specific DTOs/cursors;
it must not flatten runtime, audit, transcript, or future memory facts into a
single generic event payload. Resume helpers return domain-specific updates
when a cursor is valid, or an explicit snapshot/rebase response when retention
has made replay impossible. A cursor minted under a different scope remains an
authority failure and must not be silently converted into a snapshot.

Rules:

- event ids are scoped; a user cannot replay another user's stream
- replay gaps produce an explicit snapshot/rebase, not silent data loss
- transport-specific reconnect details do not leak into core runtime services

---

## 7. Transport adapter boundary

`TransportAdapter` owns protocol translation only.

It may own:

- HTTP/SSE/WebSocket/channel protocol details
- webhook signature verification before runtime request creation
- converting runtime events to transport payloads
- transport-specific keepalive behavior

It must not own:

- capability authorization
- prompt assembly
- approval semantics
- auth flow semantics
- durable transcript ownership
- projection source-of-truth state

---

## 8. Redaction and safety

Events must not leak:

- raw secrets
- raw host paths
- private auth tokens
- unapproved filesystem contents
- policy-denied request payloads

When an event references sensitive data, use:

- handles
- scoped paths
- redacted summaries
- correlation ids
- structured denial reasons

Durable rows and JSONL files must not add raw secrets, host paths, request payloads, runtime output, approval reasons, or backend detail strings. Connection and migration failures are reported through redacted backend/operation errors. Event constructors and serialization enforce runtime `error_kind` sanitization; producer crates remain responsible for constructing metadata-only audit envelopes.

---

## 9. Non-goals

This contract does not define the final event store backend, wire protocol, UI schema, or audit retention policy. It defines the ownership boundaries and minimum invariants needed before those implementation choices are made.

---

## Contract freeze addendum — durable streams and projections (2026-04-25)

V1 includes a durable append log with scoped replay cursors as the first event substrate. Projection and SSE/WebSocket APIs are downstream product integrations backed by that substrate; they should not block landing or testing the append-log/cursor contract.

Minimum substrate event-store contract:

```text
append redacted event
read after cursor
read scoped stream snapshot
retention/replay-gap reporting
caller-level test replay across service boundaries
```

Additional projection/transport contract:

```text
ack/track cursor where transport needs it
projection rebuild from durable events/state
SSE/WebSocket resume over validated scoped cursors
```

Cursor rules:

- cursors are monotonic within a scoped stream;
- a cursor is not global authority and must be validated against tenant/user/thread/process scope;
- replay gaps return an explicit snapshot/rebase marker, not silent loss;
- SSE/WebSocket transports resume from the last accepted cursor and then tail live events.

V1 event streams must cover at least:

```text
turn/run progress
process lifecycle/output refs
approval state
runtime invocation state
memory significant events
extension lifecycle
resource/network/security audit summaries
```

Event delivery failures are best-effort for live transports; durable append failures are domain-specific and must be explicit where the event is required audit/history.

---

## Standalone durable backend addendum

The current standalone durable backends are JSONL, PostgreSQL, and libSQL. Each stores runtime and audit streams separately, keyed by `(stream_kind, tenant_id, user_id, agent_id)`, and persists cursor envelopes so a process restart can replay from the last seen cursor.

### Profile rules

- `LocalDev` and `Test` may explicitly use in-memory stores.
- `Production` rejects in-memory stores before returning a service graph.
- `Production` may use JSONL only when the config explicitly accepts single-node durable storage.
- PostgreSQL and libSQL adapters are available behind the crate's `postgres` and `libsql` features. Their schema files live in `crates/ironclaw_reborn_event_store/migrations/`, and the factory runs those migrations before returning the service graph.
- If the crate is compiled without a requested SQL backend feature, the factory fails closed with a redacted backend-unavailable error.

### Replay semantics

Durable backends must match `InMemoryDurableEventLog` cursor behavior:

- cursors are monotonic per `(stream_kind, tenant, user, agent)`;
- `read_after_cursor(None)` starts at origin;
- `limit == 0` is rejected;
- cursors beyond the stream head return `ReplayGap`;
- retained-history gaps return `ReplayGap` rather than silent loss;
- `ReadScope` filtering is enforced by the backend;
- records filtered out by `ReadScope` still advance the scanned cursor.

### Projection boundary

Product-facing timeline, status, approval, auth, tool-call, process, resource, and memory views should be projections over durable logs, not a second source of truth. Projection services must tolerate replay gaps with explicit snapshot/rebase behavior and must not mutate control-plane state while deriving read models.

---

## Projection service addendum

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

`ProjectionReplay.updates` contains timeline entries after the supplied cursor. Its `runs` field contains the current status for runs touched by those updates, rebuilt from the scoped log prefix through `next_cursor` so paged consumers do not lose process state that was established before the current page.

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

Run lists are ordered by most recent projected activity first, with cursor ordering as a deterministic tie-breaker.

Approval/auth gates, resource/cost, memory activity, and mission progress are deferred until their source event coverage is stable. In particular, approval resolution audit exists today, but a complete `ApprovalGate` projection also needs a reliable approval-request source event or store integration.

## No-Exposure Rules

Projection DTOs must not expose raw input, raw output, secrets, raw host paths, approval reasons, invocation fingerprints, backend detail strings, or provider/runtime error payloads. They may expose stable metadata such as capability id, runtime kind, provider id, process id, output byte counts, sanitized error kind, and timestamps.

Tests must include cross-scope non-leak coverage and sentinel checks for projection output and projection error strings.
