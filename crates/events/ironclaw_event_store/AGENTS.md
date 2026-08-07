# ironclaw_event_store — working rules

Canonical crate guidance (this crate has no `CLAUDE.md`, by design — it is not
in the root module-spec table). Orientation and public surface:
[`README.md`](./README.md). Family boundary and the one-way pipeline rule:
[`../AGENTS.md`](../AGENTS.md).

## Start Here

- Read `README.md` for what the crate is; read `Cargo.toml` for actual
  dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/events.md`
- `docs/reborn/contracts/events-projections.md`
- `docs/reborn/contracts/storage-placement.md`

## What This Crate Owns

- Reborn-owned durable event/audit store backends and their selection service, currently:
- Backend selection/composition: `RebornEventStoreConfig`, `RebornProfile`, `RebornEventStores`, `RebornEventStoreError`.
- Concrete durable-log backends implementing the `ironclaw_event_log` `DurableEventLog`/`DurableAuditLog` traits: filesystem (`FilesystemDurableEventLog`, `FilesystemDurableAuditLog`), JSONL (`JsonlDurableEventLog`, `JsonlDurableAuditLog`), The crate declares **no cargo features** and has no per-backend `LibSql*`/`Postgres*` log impls — those were removed; libSQL/Postgres dispatch happens one layer down, at `RootFilesystem` (see `src/lib.rs`).
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- The PostgreSQL TLS/driver cone (`deadpool-postgres`, `tokio-postgres-rustls`)
  — but **not** in the public API. `open_postgres_pool_with_tls_options` returns
  `ironclaw_filesystem::PostgresConnectionPool`, and
  `RebornEventStoreConfig::PostgresPool` carries it, so no caller has to name
  `deadpool_postgres` (PROPOSAL §6.3.2). The driver type may only appear inside
  the **body** of the private `postgres_backed` module —
  `reborn_persistence_driver_boundary.rs::event_store_names_the_driver_only_inside_its_private_backend_module`
  scans **every** `.rs` file in `src/` minus that brace-matched body, so a
  mention in a sibling module, or after the body in `lib.rs`, fails it too.
  Keep the module **private**: `pub mod postgres_backed` (or `pub(crate)`)
  re-exports the cone the module exists to contain, and the gate rejects it by
  name. Keep the brace match honest too — it tracks line comments, nestable
  block comments, char literals, and strings **including ones that span lines**,
  because a `{` on the continuation line of a literal used to stretch the exempt
  range past the module's real end and hide every driver mention after it
  (fail-open; found 2026-08-04, see the `event_store` row in
  `docs/reborn/target-architecture/CHECKLIST.md`). An unterminated body panics
  rather than exempting the rest of the file.

## Do Not Move In Here

- product projections, transport fanout, runtime workflow policy, or backend-specific public errors.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_event_store`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- Driver-boundary check: `cargo test -p ironclaw_architecture_tests --test reborn_persistence_driver_boundary`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
