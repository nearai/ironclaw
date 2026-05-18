# Reborn EventStreamManager Assumptions

Date: 2026-05-19

Issue #3281 names product-facing concepts such as `ProductActor`, `ProductScope`, and `ProjectionTarget`, while `reborn-integration` already has strong local types for the same boundaries. This implementation maps them as follows:

- `ProductActor` -> `ironclaw_turns::TurnActor`
- explicit projection scope -> `ironclaw_event_projections::ProjectionScope`
- outbound/thread fanout scope -> `ironclaw_turns::TurnScope`
- product target -> `ProjectionTarget` in `ironclaw_event_streams`, carrying existing `ThreadId`, `MissionId`, `InvocationId`, and `ProcessId` types

The first slice keeps production transports and DB-backed projection stream stores out of scope, matching #3281. Live update delivery uses an in-memory/broadcast `ProjectionUpdateSource` test seam and composes existing projection/outbound services instead of moving their ownership.
