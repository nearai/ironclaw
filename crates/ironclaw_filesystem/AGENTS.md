# Agent Map — ironclaw_filesystem

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/filesystem.md`
- `docs/reborn/contracts/storage-placement.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- The universal storage-dispatch fabric: one trait, one entry type, one mount table behind which every persistence concern in the workspace lives. Currently:
- The single `RootFilesystem` trait (`root`) — every backend *and* the composite dispatcher implement it; no parallel "backend" trait.
- The universal stored value `Entry`/`VersionedEntry` and its primitives `RecordKind`/`RecordVersion`/`SeqNo`/`CasExpectation`/`ContentType` (`record`).
- Declarative index/query primitives `IndexSpec`/`IndexName`/`IndexKey`/`IndexValue`/`IndexKind`/`Filter`/`Page` (`index`) — no SQL strings cross the boundary; plus shared brute-force vector ranking helpers (`vector`).
- Filesystem vocabulary in `types`: `BackendCapabilities`/`BackendId`/`BackendKind`/`Capability`/`TxnCapability`, `FileStat`/`DirEntry`/`FileType`/`ContentKind`, `StorageClass`, `IndexPolicy`/`IndexConflictReason`, `FilesystemError`/`FilesystemOperation`; supporting handles `StorageTxn`/`EventRecord` (`backend`).
- Mount table + catalog: `CompositeRootFilesystem`, `MountDescriptor`, `PathPlacement` (`catalog`) — longest-prefix mount routing and inherent catalog inspection.
- Invocation-scoped view `ScopedFilesystem` + `MountViewResolver` (`scoped`) — checks permission against `MountView` before any backend dispatch.
- Backends, all implementing `RootFilesystem`: `LocalFilesystem`, `PostgresRootFilesystem`, `LibSqlRootFilesystem`, `InMemoryBackend`, `HsmBackend`; plus backend containment (symlink traversal, mount escape, raw-host-path prevention).
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- Note: this supersedes the older "bytes mount; structured records stay typed" boundary (ADR `docs/reborn/2026-05-14-universal-fs-dispatch.md`). The legacy bytes-plane methods and `src/db.rs` are transitional and slated for removal — do not add new consumers.

## Do Not Move In Here

- memory-domain path grammar, network/secrets/dispatcher behavior, and product workflow.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_filesystem`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
