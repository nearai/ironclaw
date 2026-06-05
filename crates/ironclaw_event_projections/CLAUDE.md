# ironclaw_event_projections

Product-facing read models over Reborn durable event/audit logs.

This crate is above `ironclaw_events` and below product adapters. Keep it:

- replay/materialization agnostic: expose projection traits and DTOs, not backend rows;
- metadata-only: never add raw inputs, raw outputs, host paths, secrets, approval reasons, invocation fingerprints, or backend detail strings to projection output;
- scoped: all reads must carry explicit stream and read-scope filters;
- non-mutating: projection failures must not mutate durable logs or kernel state;
- backend-independent: do not depend on JSONL/PostgreSQL/libSQL adapter crates directly.

Current slices:

- replay-derived `ThreadTimeline` and `RunStatusProjection` over `DurableEventLog`;
- `PendingGateProjection`, a Reborn turn-event read model over typed blocked/resume/terminal `TurnLifecycleEvent` facts.

The pending-gate projection may carry stable metadata needed to key the product read model: tenant, owner, thread, run, gate kind, opaque gate ref for the resolver, and blocked timestamp. Do not add approval reasons, raw prompts, tool input, backend errors, or host paths to the projection row.

Legacy root `src/gate/PendingGateStore` writers still exist for the pre-Reborn engine path. Treat them as legacy engine owners until a composition adapter maps `PendingGateProjectionSink` rows into that store; do not add new direct Reborn pending-gate writers outside the projection consumer.

See `PENDING_GATE_PROJECTION.md` for the current writer audit and follow-up
composition boundary.
