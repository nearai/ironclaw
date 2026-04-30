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
