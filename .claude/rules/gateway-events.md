---
paths:
  - "crates/ironclaw_events/**"
  - "crates/ironclaw_event_projections/**"
  - "crates/ironclaw_event_streams/**"
  - "crates/ironclaw_reborn_event_store/**"
  - "crates/ironclaw_product_workflow/**"
  - "crates/ironclaw_webui/**"
  - "src/channels/web/**"
---
# Reborn events and transport projections

Durable typed events are the source of truth for replayable event history.
Product projections derive readable state from those events, and transport
streams deliver projection updates. Domain stores may own authoritative domain
state; their event append must follow an explicit consistency/ordering contract.
HTTP, SSE, WebSocket, and product adapters must not invent a parallel state
transition that cannot be replayed.

Re-derive the current ownership before changing the path:

```bash
rg -n "RuntimeEvent|EventLogEntry|Projection|StreamManager|subscribe|replay" \
  crates/ironclaw_events crates/ironclaw_event_projections \
  crates/ironclaw_event_streams crates/ironclaw_reborn_event_store
```

## Rules

- Persist the canonical event before advertising replayable state, following
  the owning domain's consistency/ordering contract.
- Projection services own projection models, scope-filtered reads, and
  projection cursors.
- Stream managers own access/admission, redaction validation, live/replay
  stitching, bounded delivery, lag, and rebase signals.
- Transport crates own framing and keepalives only.
- Ephemeral token chunks or heartbeats must be explicitly typed as ephemeral;
  they cannot masquerade as reconstructible product state.
- Never send raw runtime payloads through a product stream. Project into a
  redacted contract first.
- A reconnect must recover from persisted events/projections, not from process
  memory or optimistic frontend state.

## Required path

For a new durable UI state transition:

1. Define or reuse a typed, redacted event in the owning contract.
2. Append it through the durable event sink in the order/consistency model
   defined by the owning domain; do not imply atomic cross-store commit unless
   the implementation guarantees it.
3. Extend the projection service with scope filtering and cursor semantics.
4. Let `EventStreamManager` validate redaction and stitch replay with live
   delivery.
5. Translate the projected contract into transport framing at the edge.
6. Make the frontend reconcile from the projected snapshot/replay contract.

Forbidden shortcuts include direct handler broadcasts of durable-looking state,
raw runtime output in transport events, in-memory channels as the only record of
a transition, cursors advanced before append succeeds, client-provided scope
controlling visibility, and reconnect paths that ignore replay/rebase.

## Ephemeral transport data

Heartbeats and model-token chunks may be transport-only because they claim no
durable product state. Mark them with an explicit ephemeral type and keep them
out of durable projections. Any additional transport-only category needs a
written reason why replay and reconciliation are meaningless for it.

## Verification

There is no annotation that makes a direct broadcast safe. Review the whole
producer-to-consumer path. Re-derive it with:

```bash
rg -n "append|EventSink|EventLog" crates/ironclaw_events crates/ironclaw_reborn_event_store
rg -n "ProjectionRequest|ProjectionCursor|snapshot|updates" crates/ironclaw_event_projections
rg -n "EventStreamManager|subscribe|rebase|lag|redaction" crates/ironclaw_event_streams
rg -n "Sse|WebSocket|stream" crates/ironclaw_webui
```

Durable event variants require tests for persistence, replay, projection visibility,
redaction, ordering, lag/rebase behavior, and transport serialization at the
appropriate public seams.
Ephemeral variants require serialization, ordering, and transport coverage, but
must not be tested as persisted or replayable state. Apply visibility,
redaction, and lag/rebase assertions wherever their transport contract exposes
those behaviors.
