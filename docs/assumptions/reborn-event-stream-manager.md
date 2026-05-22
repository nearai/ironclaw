# Reborn EventStreamManager Assumptions

Date: 2026-05-19

Issue #3281 names product-facing concepts such as `ProductActor`, `ProductScope`, and `ProjectionTarget`, while `reborn-integration` already has strong local types for the same boundaries. This implementation maps them as follows:

- `ProductActor` -> `ironclaw_turns::TurnActor`
- explicit projection scope -> `ironclaw_event_projections::ProjectionScope`
- outbound/thread fanout scope -> `ironclaw_turns::TurnScope`, paired with the actor, projection scope, view, and target used for access authorization before fanout candidates are planned
- product target -> `ProjectionTarget` in `ironclaw_event_streams`, carrying existing `ThreadId`, `MissionId`, `InvocationId`, and `ProcessId` types

The first slice keeps production transports and DB-backed projection stream stores out of scope, matching #3281. Live update delivery uses an in-memory/broadcast `ProjectionUpdateSource` test seam and composes existing projection/outbound services instead of moving their ownership.

Review hardening added host-owned limits because the issue does not define final production sizing:

- Product subscribers may request a buffer capacity, but the host caps it at 128 items and clamps zero to one item.
- Redaction-validation decisions are cached per envelope kind, scope, runtime cursor, payload length, and payload digest. The in-memory cache is capped at 1024 decisions and evicts incrementally by recency on overflow; a later durable/multitenant stream host can replace that policy with a tenant-aware LRU if needed.
