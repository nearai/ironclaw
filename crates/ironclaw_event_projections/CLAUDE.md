# ironclaw_event_projections

Product-facing read models over Reborn durable event/audit logs.

This crate is above `ironclaw_events` and below product adapters. Keep it:

- replay/materialization agnostic: expose projection traits and DTOs, not backend rows;
- metadata-only: never add raw inputs, raw outputs, host paths, secrets, approval reasons, invocation fingerprints, or backend detail strings to projection output;
- scoped: all reads must carry explicit stream and read-scope filters;
- non-mutating: projection failures must not mutate durable logs or kernel state;
- backend-independent: do not depend on JSONL/PostgreSQL/libSQL adapter crates directly.

Current first slice is replay-derived `ThreadTimeline` and `RunStatusProjection` over `DurableEventLog`. Approval/auth/resource/memory/mission projections should be added only when their source facts are stable and typed.
