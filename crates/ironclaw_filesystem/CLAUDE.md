# ironclaw_filesystem guardrails

`ironclaw_filesystem` is the **universal storage dispatch fabric** for IronClaw.
There is one trait (`RootFilesystem`), one entry type (`Entry`), one mount
table (`CompositeRootFilesystem`). Every persistence concern in the workspace
(secrets, leases, processes, memory documents, project files, event logs,
engine state, settings, …) lives behind a single set of ops: `put` / `get` /
`delete` / `list_dir` / `query` / `ensure_index` / `stat` / `begin` /
`append` / `tail`.

This supersedes the earlier "bytes mount; structured records stay typed"
boundary recorded in
`docs/reborn/2026-04-25-storage-catalog-and-placement.md`. The override is
codified in `docs/reborn/2026-05-14-universal-fs-dispatch.md` (the new ADR).

## What this crate owns

- `RootFilesystem` (`src/root.rs`) — the one trait every backend and the
  composite dispatcher implement.
- `Entry` / `VersionedEntry` / `RecordKind` / `RecordVersion` / `SeqNo` /
  `CasExpectation` / `ContentType` (`src/record.rs`) — the universal stored
  thing and its associated primitives.
- `IndexSpec` / `IndexName` / `IndexKey` / `IndexValue` / `IndexKind` /
  `Filter` / `Page` (`src/index.rs`) — declarative index/query primitives.
  No SQL strings cross this boundary.
- `BackendCapabilities` / `IndexCapability` / `TxnCapability`
  (`src/types.rs`) — declared up front; mount-time validation refuses a
  backend that cannot serve what a consumer demands.
- `StorageTxn` / `EventRecord` (`src/backend.rs`) — supporting handle types.
- `CompositeRootFilesystem` / `MountDescriptor` / `FilesystemCatalog`
  (`src/catalog.rs`) — the longest-prefix mount table.
- `ScopedFilesystem` (`src/scoped.rs`) — the invocation-scoped view that
  higher-level stores accept in their constructor. Performs the permission
  check against `MountView` before any backend dispatch.
- Backends: `LocalFilesystem`, `PostgresRootFilesystem`,
  `LibSqlRootFilesystem`, `InMemoryBackend`. All implement
  `RootFilesystem`.
- Backend containment checks (symlink traversal, mount escape, raw-host
  path prevention).

## What this crate does NOT do

- Define a separate "backend trait" parallel to `RootFilesystem`. There is
  one trait. `CompositeRootFilesystem` is itself a `RootFilesystem` that
  dispatches by mount; there is no two-tier `Backend` / `Dispatcher`
  split. Adding a parallel trait is exactly the duplication this rework
  removed.
- Own product-shaped paths or schemas. Path conventions (`/secrets/...`,
  `/memory/...`, `/engine/threads/...`) live in the consumer crates.
- Hold raw host paths in public types. `HostPath` stays backend-internal
  and is not serializable.
- Depend on `ironclaw_*` system-service or runtime crates other than
  `ironclaw_host_api` and `ironclaw_safety`.

## Invariants new code must preserve

1. **One trait, one Entry, one dispatch fabric.** Adding a parallel
   trait/type to handle a special case is a sign to either widen `Entry`
   or extend `RootFilesystem`. Don't fork.
2. **CAS is the floor.** Every multi-step store operation (claim, consume,
   transition) is implemented with `put(_, _, CasExpectation::Version)` +
   retry on `FilesystemError::VersionMismatch`. Consumers must never assume
   `begin`/`StorageTxn` is available; backends that don't expose it return
   `Unsupported` and that must be a working path.
3. **Capabilities are declared, not discovered.** A backend that cannot
   serve an `IndexKind::Vector` or a `Filter::Range` declares so up front
   via `BackendCapabilities`; mount-time validation refuses the attachment.
   Runtime `Unsupported` errors are a fallback, not the primary signal.
4. **Indexed projection is the only queryable surface.** Backends never
   parse `Entry::body` to evaluate filters. Everything queryable lives in
   `Entry::indexed`. This keeps the indexing contract portable across SQL,
   filesystem-sidecar, and HSM backends.
5. **Encryption-at-rest is a backend decorator.** `EncryptedBackend`
   (forthcoming) wraps an inner backend and encrypts `Entry::body` plus any
   `IndexValue::Bytes` projection while letting scalar indexed projections
   (`scope`, `status`, …) pass through unencrypted. `SecretStore` and other
   sensitive-data stores never own encryption code — they write plaintext
   `Entry` values through a `ScopedFilesystem` whose mount happens to be
   wrapped in encryption.
6. **No raw host paths leak.** Backends translate `VirtualPath` /
   `ScopedPath` to host paths internally and never carry host paths in
   public types or error display output.
7. **Tenant/user virtual-path scoping is preserved.** Multi-tenant
   deployments rely on the path prefix to route to per-tenant mounts. New
   persistence behavior must keep the scope keys in the path, not
   exclusively in `Entry::indexed`.

## Legacy bytes plane (transitional)

`read_file` / `write_file` / `append_file` / `list_dir` / `stat` /
`delete` / `create_dir_all` remain on `RootFilesystem` during the
migration window. Default impls route reads/writes through `put`/`get` so
existing backends (and existing consumer code) continue to work without
changes. These methods will be removed entirely once
`src/db/` is dissolved (task #17 of the storage rework). Do not add new
consumers of the legacy methods — new code should call `put`/`get`/
`query`/etc. directly.

## When you're editing this crate

- Run the full crate tests, both feature combinations:
  `cargo test -p ironclaw_filesystem --all-features`,
  `cargo check -p ironclaw_filesystem --no-default-features --features libsql`,
  `cargo check -p ironclaw_filesystem --no-default-features --features postgres`.
- New `Entry` shapes (record kinds, indexed projections) belong in the
  consumer crate, not here. This crate only owns the trait surface and
  shared primitives.
- New backend implementations live as siblings under `src/` and implement
  `RootFilesystem`. Declare capabilities accurately; the mount table
  enforces them.
- Any change to the trait surface needs an accompanying
  `InMemoryBackend` test demonstrating the new op in
  `src/in_memory.rs::tests`.
