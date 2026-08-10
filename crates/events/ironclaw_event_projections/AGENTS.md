# ironclaw_event_projections — working rules

Canonical crate guidance (the crate's `CLAUDE.md` is a pointer here).
Orientation and public surface: [`README.md`](./README.md). Family boundary
and the one-way pipeline rule: [`../AGENTS.md`](../AGENTS.md).

## Start here

- Read `README.md` for what the crate is; read `Cargo.toml` for actual
  dependencies and feature shape.
- Contract docs that outrank intuition: `docs/reborn/contracts/events.md`,
  `docs/reborn/contracts/events-projections.md`; neighbors:
  `crates/events/ironclaw_event_log/AGENTS.md`,
  `crates/events/ironclaw_event_streams/AGENTS.md`.

## Working rules

- **Replay/materialization agnostic:** expose projection traits and DTOs
  (`EventProjectionService`/`ReplayEventProjectionService`,
  `AuditProjectionService`/`ReplayAuditProjectionService`), never backend
  rows; stay independent of JSONL/PostgreSQL/libSQL adapters.
- **Metadata-only:** never add raw inputs, raw outputs, host paths, secrets,
  approval reasons, invocation fingerprints, or backend detail strings to
  projection output. New DTO fields must remain metadata-only and explicitly
  scoped.
- **Scoped:** all reads carry explicit stream and read-scope filters
  (`ProjectionScope`, `ReadScope`).
- **Non-mutating:** projection failures are observable, never mutating — no
  write path into durable logs or kernel state, under any name. The dependency
  surface enforces this (see boundary below); keep it that way.
- **No second stream manager:** subscription, admission, and live delivery
  belong exclusively to `ironclaw_event_streams`. This crate once carried a
  name-colliding `EventStreamManager`, a `DurableMemoryAuditSink` (a write
  path), and a dead `PendingGateProjection`; all three were deleted per
  PROPOSAL §6.3.3 — do not reintroduce any of them here.
- **Bounded replay:** respect the replay page limit and the rebase ceiling; a
  consumer past the ceiling gets an explicit rebase-required error and must
  request a fresh snapshot rather than assume entries were silently skipped.

## The one display exception, and who owns its sanitization

`CapabilityActivityProjection.error_detail` may carry only the sanitized
`RuntimeEvent` error summary. It is still not a general backend-detail
channel; raw tool input/output, host paths, secrets, and provider messages
that fail the runtime-event sanitizer must remain collapsed to the fixed safe
summaries. Ownership ladder:

- runtime producers pass only host-authored summaries into
  `RuntimeEvent::with_error_summary`;
- `ironclaw_event_log` owns durable-log sanitization at construction,
  serialization, and deserialization boundaries;
- this crate **re-runs** `ironclaw_event_log::sanitize_error_summary` when
  deriving `error_detail` (see `runtime_projection::projection_error_detail`),
  because product projections are a separate user-facing boundary;
- product workflow and WebUI treat `error_detail` as already display-bounded
  and must not recover or append raw backend detail.

## Dependency boundary

Internal deps (normal): `ironclaw_event_log` + `ironclaw_host_api` — exactly
the §6.3.3 target shape. The `ironclaw_event_projections` `BoundaryRule` in
`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
forbids `ironclaw_event_store` and `ironclaw_filesystem` among the whole
privileged tier, which is what makes "projections never write" structural.
Dev-deps pull in the real store + filesystem (`test-support`) for
source-fault-path tests only.

## Validation

- Fast local check: `cargo test -p ironclaw_event_projections`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
- Run outbound/product workflow tests when projection shape changes affect
  delivery candidates or UI-visible feeds.
