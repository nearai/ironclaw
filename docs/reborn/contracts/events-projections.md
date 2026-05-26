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
- capability/tool activity lifecycle metadata
- job list
- project/thread visibility
- extension capability surface
- approval/auth pending gates
- harness/check status

Reducer rules:

- deterministic for the same input state/events
- side-effect free
- rebuildable after restart
- product-facing capability activity projections expose only metadata-safe
  lifecycle facts; raw tool arguments, raw output, command strings, host paths,
  and provider payloads stay outside the projection contract
- transport adapters may bound activity fan-out per projection item; bounded
  snapshots must prefer the most recently updated activity facts and keep
  reconnect cursors resumable
- may cache output, but cache is not source of truth
- must tolerate unknown future event types by ignoring or preserving them according to version policy

Projection crates may consume typed domain-event contracts from the owning
domain crate when the read model is explicitly about that domain. For example,
`ironclaw_event_projections` may consume `ironclaw_turns::TurnLifecycleEvent`
to derive pending-gate rows from blocked/resumed/terminal turn facts. That does
not make projections the source of truth for turn state, and it does not permit
imports from product composition, root `src/`, or legacy engine pending-gate
types.

Read-model keys must preserve the full scope needed to enforce read isolation.
Turn-derived pending-gate rows preserve tenant, agent, project, owner, thread,
and run identity; a sink must not collapse rows to tenant/thread when the source
`TurnScope` carries narrower agent/project boundaries.

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

`ironclaw_event_projections::EventStreamManager` is the lower-level
transport-agnostic facade over domain projection services. It routes scoped
runtime and audit replay requests through their owning projection services and
preserves their domain-specific DTOs/cursors. Resume helpers return
domain-specific updates when a cursor is valid, or an explicit snapshot/rebase
response when retention has made replay impossible. A cursor minted under a
different scope remains an authority failure and must not be silently converted
into a snapshot.

`ironclaw_event_streams::EventStreamManager` is the product-facing stream
manager slice. It composes projection snapshots/replay with actor+scope access
policy, stream admission, bounded live update buffering, stream-boundary
redaction validation, transport-neutral stream items (`Snapshot`, `Update`,
`RebaseRequired`, `Lagged`, `KeepAlive`), and separate outbound push-candidate
selection through `ironclaw_outbound`. It consumes projection DTOs and outbound
policy ports only; it must not read durable event/audit stores directly, own
reducers, render transport frames, or infer external push permission from
subscription visibility.

Rules:

- event ids are scoped; a user cannot replay another user's stream
- replay gaps produce an explicit snapshot/rebase, not silent data loss
- truncated snapshot/replay pages produce an explicit lag signal before live tailing, not a cursor treated as complete
- access policy runs before snapshot, replay, or live subscription
- long-lived subscriptions pass admission policy and use bounded buffering
- external push/fanout candidates are selected separately from subscribers and require projection access authorization for the actor/scope/target being fanned out
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

Run status projections are projection-local read models. Model/reply milestone events use this status mapping:

- `ModelStarted`, `ModelCompleted`, and `ModelFailed` keep the run `running`; model completion means the provider returned, and model failure is attempt-level progress that may be retried/recovered.
- `AssistantReplyFinalized` marks the run `completed` for metadata-only loop model/reply runs until richer terminal turn-run events are available.
- `LoopCompleted` and `LoopFailed` are trusted terminal loop milestones; they mark the run `completed` or `failed` with sanitized `error_kind` only.

### Outbound egress and subscription state

`ironclaw_outbound` owns the metadata-only state and policy seams needed around projection delivery:

- per-thread notification policy for explicit external fanout and progress opt-in;
- durable projection subscription cursor checkpoints scoped to actor, thread, and `ProjectionScope`;
- a projection access-policy port that authorizes actor/thread visibility before subscription cursor state is created;
- a reply-target validation port used before each external push candidate is turned into a delivery attempt;
- outbound delivery attempt/status rows for support-visible retry/dead-letter workflows.

This state is not canonical transcript or projection content. Rows store refs, cursors, status enums, timestamps, and sanitized failure kinds only. Product adapters still revalidate reply-target binding authorization before every external push, and delivery failure must not mutate canonical transcript/projection state or mark turns/runs failed. If reply-target authorization is revoked at delivery time, the outbound policy service records a sanitized `authorization_revoked` delivery failure and does not return a sendable target.
