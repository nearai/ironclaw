# ironclaw_event_projections

Product-facing read models over Reborn durable event/audit logs.

This crate is above `ironclaw_events` and below product adapters. Keep it:

- replay/materialization agnostic: expose projection traits and DTOs, not backend rows;
- metadata-only: never add raw inputs, raw outputs, host paths, secrets, approval reasons, invocation fingerprints, or backend detail strings to projection output;
- scoped: all reads must carry explicit stream and read-scope filters;
- non-mutating: projection failures must not mutate durable logs or kernel state;
- backend-independent: do not depend on JSONL/PostgreSQL/libSQL adapter crates directly.

The one allowed product-display exception is `CapabilityActivityProjection.error_detail`:
it may carry only the sanitized `RuntimeEvent.error_summary` value after replay
re-runs `ironclaw_events::sanitize_error_summary`. This field is still not a
general backend-detail channel; raw tool input/output, host paths, secrets, and
provider messages that fail the runtime-event sanitizer must remain collapsed to
the fixed safe summaries.

Sanitization ownership for this exception is:

- runtime producers should pass only host-authored summaries into
  `RuntimeEvent::with_error_summary`;
- `ironclaw_events` owns durable-log sanitization at construction,
  serialization, and deserialization boundaries;
- `ironclaw_event_projections` must re-run the same sanitizer when deriving
  `error_detail`, because product projections are a separate user-facing
  boundary;
- product workflow and WebUI layers must treat `error_detail` as already
  display-bounded and must not recover or append raw backend detail.

Current slices:

- replay-derived `ThreadTimeline` and `RunStatusProjection` over `DurableEventLog`;
- `PendingGateProjection`, a Reborn turn-event read model over typed blocked/resume/terminal `TurnLifecycleEvent` facts.

The pending-gate projection may carry stable metadata needed to key the product read model: tenant, owner, thread, run, gate kind, opaque gate ref for the resolver, and blocked timestamp. Do not add approval reasons, raw prompts, tool input, backend errors, or host paths to the projection row.

Legacy root `src/gate/PendingGateStore` writers still exist for the pre-Reborn engine path. Treat them as legacy engine owners until a composition adapter maps `PendingGateProjectionSink` rows into that store; do not add new direct Reborn pending-gate writers outside the projection consumer.

See `PENDING_GATE_PROJECTION.md` for the current writer audit and follow-up
composition boundary.
