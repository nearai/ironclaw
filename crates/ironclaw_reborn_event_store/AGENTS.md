# Agent Map — ironclaw_reborn_event_store

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the Reborn contracts below.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/events.md`
- `docs/reborn/contracts/events-projections.md`
- `docs/reborn/contracts/storage-placement.md`

## What This Crate Owns

- Reborn-owned durable event/audit store backends and their selection facade, currently:
- Backend selection/composition: `RebornEventStoreConfig`, `RebornProfile`, `RebornEventStores`, `RebornEventStoreError`.
- Concrete durable-log backends implementing the `ironclaw_events` `DurableEventLog`/`DurableAuditLog` traits: filesystem (`FilesystemDurableEventLog`, `FilesystemDurableAuditLog`), JSONL (`JsonlDurableEventLog`, `JsonlDurableAuditLog`), and the feature-gated libSQL/Postgres backends (behind the `libsql` / `postgres` features).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- product projections, transport fanout, runtime workflow policy, or backend-specific public errors.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_reborn_event_store`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
