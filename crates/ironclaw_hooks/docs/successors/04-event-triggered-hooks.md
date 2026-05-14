# Successor PR: event-triggered hooks (Phase 5)

> Successor work from PR #3573. The current hook framework fires at
> inline dispatch points (`before_capability`, `before_prompt`,
> `after_model`, `after_capability`, `after_checkpoint`). Phase 5 of
> the original roadmap adds **event-triggered hooks**: hooks that
> subscribe to the runtime event bus and react to durable
> `RuntimeEvent`s asynchronously, outside the loop's inline tick.

## Motivation

Inline hook points are synchronous against the loop's dispatch path —
they observe and gate, but they fire *during* the loop's own work.
Some legitimate hook use cases don't fit that shape:

- **Cross-run policy enforcement**: a hook that audits "ext-A has
  made >10 polymarket trades across all runs in the last 24h" can't
  evaluate against per-run state. It needs the event log.
- **Asynchronous notifications**: a hook that emits a Slack ping when
  a `HookFailed` event fires shouldn't block the loop on Slack's
  HTTP latency.
- **Post-hoc analysis / fan-out**: a hook that pipes finalized model
  responses into an embedding index, a search log, or a downstream
  policy engine.

These are observer-only by construction: they read durable events
and produce side effects but never gate the originating loop (which
has already finished).

## Scope

1. New point type `EventTriggered` registered against a
   `RuntimeEventKind` filter (e.g., "fire on
   `RuntimeEventKind::HookFailed` for any hook in tenant α").
2. Subscription is per-build (or per-process, if cross-run state is
   needed — design discussion required).
3. Hook receives a typed `EventHookContext { event: &RuntimeEvent,
   replay_cursor }`. Same trust-tier rules apply (Installed /
   Trusted / Builtin), but `Effect` and `Gate` decisions are
   **not** available — event-triggered hooks are observer-only.
4. The hook framework's existing failure-policy matrix applies:
   panic / timeout / malformed → `FailureCategory::*`, isolated from
   the originating loop.

## Likely surface

```rust
// in ironclaw_hooks::points::event_triggered
pub struct EventHookContext<'a> {
    pub event: &'a RuntimeEvent,
    pub event_cursor: EventCursor,
    pub tenant_id: TenantId,
}

// Sink mirrors ObserverSink — `note_fact`, `emit_audit`. No `allow`,
// no `deny`, no `patch`.
#[async_trait]
pub trait EventTriggeredHook: Send + Sync {
    async fn handle(
        &self,
        ctx: &EventHookContext<'_>,
        sink: &mut dyn ObserverSink,
    );
}
```

Subscription is via manifest:

```toml
[[hooks]]
id = "polymarket-fail-alert"
kind = "event_triggered"
scope = "own_capabilities"
phase = "telemetry"
[hooks.body]
mode = "predicate"
[hooks.body.spec.EventTriggeredAlert]
when.AnyOf = [
    { event_kind = "hook_failed", capability = "polymarket.place_order" }
]
emit_audit.summary = "polymarket hook failed"
```

## Architecture seams

- **Subscription side**: a new dispatcher path
  `dispatch_event_triggered_at(EventCursor, RuntimeEvent)`. The
  reborn factory wires it to `ironclaw_events`'s event-stream
  consumer (`EventStreamSubscriber`-like trait).
- **Backpressure**: event-stream consumers must not block the
  event-emit path. The hook dispatcher reads events at its own pace
  (tick-driven or stream-driven, TBD).
- **Cursor / replay**: subscriptions are cursor-keyed so a restarted
  host can resume from the last-seen `EventCursor`. Lost events
  during downtime is acceptable for observer-only semantics;
  exact-once delivery is a future ratification slice.

### Phase 5 implementation notes

- The Reborn wiring uses a pull-driven durable-log consumer rather than
  dispatching from the event emit path. The default poll interval is 50ms and
  the default replay batch is 64 events; callers can tune both on
  `EventTriggeredHookSubscription`.
- Cursor persistence remains caller-owned for this slice: the subscription
  starts from the supplied `EventCursor` and advances its in-memory cursor
  while the host is alive. Restarting from the same cursor intentionally
  gives at-least-once replay; exact-once acknowledgement is deferred.
- Hook dispatch receives the durable `RuntimeEvent` directly. This uses only
  the sealed event/cursor vocabulary from `ironclaw_events`; the hook crate
  still does not depend on host runtime, dispatcher, secrets, network, WASM,
  or Reborn internals.

## Cross-cutting constraints

- **Cross-crate boundary**: `ironclaw_hooks` already forbids `events`
  / `host_runtime` / `network` deps. Event-triggered hooks need *some*
  access to `RuntimeEvent` — either via a re-export from
  `ironclaw_events` (low risk; types are sealed-vocab) or via a
  narrowed `HookObservableEvent` projection that strips dispatcher-
  internal fields. **Pick narrowed projection** unless the full
  surface is needed.
- **Trust class**: Installed-tier event-triggered hooks default to
  the same `OwnCapabilities` scope filter as inline hooks (the event
  has `provider: Option<ExtensionId>` already, so this is a thin
  reuse of the existing scope-filter code).
- **No re-emission**: event-triggered hooks must not be allowed to
  emit a new `RuntimeEvent` that would re-trigger themselves. The
  observer-only restriction handles this by construction (sinks
  can `note_fact` / `emit_audit` but those are scoped to the hook's
  own audit substrate, not the runtime event log).

## What this PR does NOT do

- Cross-run state aggregation (the "ext-A made >10 trades across runs"
  example). That requires durable cross-run state, which depends on
  the persistent-counter slice (PR #3635) being committed and a
  cross-run query API. Phase 6 follow-up.
- WASM-bodied event hooks. Reuses the same `Wasm` body issue as the
  inline-hook WASM runtime (PR #3634); cross-referenced.
- Effect hooks (durable mutations triggered by events). Out of scope
  for this slice; observer-only.

## Required tests (caller level)

1. **Subscription matching**: hook registered against
   `RuntimeEventKind::HookFailed` fires when a hook fails and is
   silent for any other kind.
2. **Cursor resume**: after a host restart, the subscription resumes
   at the last persisted cursor and replays missed events through
   the hook.
3. **Scope filter**: `OwnCapabilities` event hook fires for events
   where `event.provider == binding.owning_extension`, doesn't fire
   for foreign providers (mirrors inline-hook scope semantics).
4. **Observer-only enforcement**: type system refuses to compile a
   hook that calls `sink.deny(...)` (the trait doesn't expose it).
5. **Backpressure**: a slow event-triggered hook doesn't block the
   loop's event-emit path. Drive with a recorder-style test.
6. **Cross-crate boundary**: `ironclaw_architecture` test confirms
   `ironclaw_hooks` doesn't gain a forbidden dep on
   `ironclaw_events` or `ironclaw_host_runtime`.

## Threat-model notes

- Event-triggered hooks see runtime events that may carry
  sanitized but real PII. The existing event sanitization
  (`sanitize_error_kind`) applies; the new path doesn't bypass it.
- Subscription DoS: an extension that subscribes to a high-volume
  event kind can flood its hook dispatcher. Mitigate via the
  existing per-extension hook-count caps (D3/D4) plus a per-hook
  per-second budget specific to event-triggered hooks (new).

## Risk

- Cross-crate dep direction: `ironclaw_events` is already a
  dependency of `ironclaw_hooks` (via the milestone projection in
  PR #3573), so the dep direction is established. The new traffic is
  the *consumer-side* of the event stream, which means
  `ironclaw_hooks` learns about `EventStreamSubscriber` or whatever
  trait `ironclaw_events` exposes.
- Coordination with PR #3635 (persistent counter): event-triggered
  hooks that need to aggregate across runs depend on the durable
  counter being available. This PR can ship the inline subscription
  + dispatch infrastructure first; aggregation lands later.
- Coordination with #3567 (self-authored hooks durable
  ratification): event-triggered hooks authored by the agent should
  go through the same ratification path. Reference the channel-to-
  user design when it lands.

## Effort

Medium. The dispatcher / subscription / cursor machinery is the
main slice. The actual hook trait + sink are thin (mirrors observer
slot).
