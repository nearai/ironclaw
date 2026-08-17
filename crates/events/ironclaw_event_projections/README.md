# ironclaw_event_projections

Replay-derived, metadata-only read models over the durable event and audit
logs, with scope, cursor, and rebase semantics — never a materialized store,
never authority. It is a separate crate so that "projections never write" is
enforced by what the crate is permitted to link rather than by review: its
dependency surface contains nothing to write with, and keeping folding isolated
from stream subscription means a projection failure can never touch a live
subscription.

- **Family / layer:** `crates/events/` / `substrates` · **Package:** `ironclaw_event_projections` · **Manifest:** `crates/events/ironclaw_event_projections/Cargo.toml`
- **Use this when:** a consumer needs "what happened" folded into a typed,
  scoped view — a thread timeline, run status, capability activity.
- **Don't use this when:** you need subscription/admission/live delivery (→
  `ironclaw_event_streams`), durable persistence (→ `ironclaw_event_store`
  via composition), or user-facing view assembly (→ `ironclaw_assistant`).

## Public surface

One flat `lib.rs` plus two private modules (`runtime_projection`,
`runtime_checkpoint_cache`):

- Service traits and their replay implementations: `EventProjectionService` /
  `ReplayEventProjectionService`, `AuditProjectionService` /
  `ReplayAuditProjectionService` — scoped by tenant, actor, and read scope.
- Request/cursor/snapshot/replay vocabulary: `ProjectionScope`,
  `ProjectionRequest`/`Cursor`/`Snapshot`/`Replay` and the audit variants; a
  bounded replay page size and a rebase ceiling past which a consumer must
  request a fresh snapshot (an explicit rebase-required error, never silent
  skipping).
- Read-model DTOs: `ThreadTimeline`/`TimelineEntry`,
  `RunStatusProjection`/`RunProjectionStatus`,
  `CapabilityActivityProjection`; `pub use
  ironclaw_event_log::EventCursor`.

## Depends on / consumed by

- **Internal deps (normal):** `ironclaw_event_log` + `ironclaw_host_api` —
  and nothing else, exactly the §6.3.3 target shape (dev-deps add the real
  store + filesystem for fault-path tests).
- **Consumed by 7 workspace crates + the integration-test root** (reproduce:
  `grep -rl '^ironclaw_event_projections = ' --include=Cargo.toml crates Cargo.toml`):
  `ironclaw_event_streams`, `ironclaw_outbound`, `ironclaw_assistant`,
  `ironclaw_host_runtime`, `ironclaw_processes`, `ironclaw_turn_runner`,
  `ironclaw_composition`.

## Invariants

- **Provably non-writing.** The `BoundaryRule` in
  `reborn_dependency_boundaries.rs` forbids `ironclaw_event_store` and
  `ironclaw_filesystem` (among the whole privileged tier), so this crate
  cannot link a durable writer. A projection failure is observable, never
  mutating.
- **Metadata-only output.** Never add raw inputs/outputs, host paths, secrets,
  approval reasons, invocation fingerprints, or backend detail strings to a
  projection DTO. The one display exception:
  `CapabilityActivityProjection.error_detail` may carry only the sanitized
  `RuntimeEvent` error summary, and replay re-runs
  `ironclaw_event_log::sanitize_error_summary` when deriving it — product
  layers treat it as display-bounded and never append backend detail.
- **Scoped reads only** — every read carries explicit stream and read-scope
  filters.
- **No second stream manager** — subscription and admission belong exclusively
  to `ironclaw_event_streams` (this crate's old `EventStreamManager`,
  `DurableMemoryAuditSink`, and `PendingGateProjection` are deleted, per
  PROPOSAL §6.3.3).

## Tests

```bash
cargo test -p ironclaw_event_projections
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance;
  `CLAUDE.md` points here).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.3.3;
  `docs/internal/reborn/target-architecture/families/events.md`; frozen contract
  `docs/internal/reborn/contracts/events-projections.md`.
