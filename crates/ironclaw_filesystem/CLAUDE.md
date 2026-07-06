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
- `CompositeRootFilesystem` / `MountDescriptor` / `PathPlacement`
  (`src/catalog.rs`) — the longest-prefix mount table and inherent catalog inspection.
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

   **Use the shared `cas_update` helper — never a per-record mutex.** Every
   filesystem read-modify-write MUST go through
   [`ironclaw_filesystem::cas_update`](src/cas.rs) (bounded CAS-retry +
   jittered backoff + overall timeout + fail-closed capability gate). Do NOT
   wrap a filesystem RMW in a per-record `tokio::sync::Mutex`
   (`FILESYSTEM_RECORD_LOCKS`-style) held across the backend `get`/`put`
   `.await`: it is a redundant in-process serializer over a backend that
   already does versioned CAS, and under burst it convoys every same-scope
   writer behind one stalled writer (the 2026-06-24 runtime wedge). It also
   tends to leak (a strong-`Arc` map that is never pruned). A handful of
   pre-existing stores still guard their CAS loop with a per-key mutex map
   instead of going lock-free; those are tracked for the same migration and
   are not a model to copy. All NEW mount-backed durable read-modify-write
   code — and every store already brought under the `cas_update` migration
   — must go through `cas_update`; do not re-copy the retry/backoff loop
   into a store. `cas_update` fails **closed** on a non-CAS backend
   (`CasUpdateError::CasUnsupported`) rather than falling back to a blind
   `CasExpectation::Any` overwrite; all production store mounts resolve to
   CAS-capable db/in-memory backends (`LocalFilesystem` is byte-only and is
   structurally unreachable from those mounts), so fail-closed is correct.
   See `docs/plans/2026-06-25-cas-migration.md`.

   **Pre-existing lock-free CAS retry loops (illustrative, not exhaustive):**
   several stores predate the `cas_update` helper and still drive their own
   local `put_with_cas` retry loop instead of calling it. The entries below
   are the ones actively tracked for migration today — they exist to orient
   you to the pattern, not as a complete inventory of every such site in the
   workspace. Do not copy any of these loops as a precedent for new code,
   and do not treat a store's absence from this list as license to write a
   new local retry loop instead of calling `cas_update`.

   - `ironclaw_turns` runner-lease sidecar (`filesystem_store/runner_lease.rs`,
     landed independently in #5232) drives its per-run lease records through
     a local `put_with_cas` + `cas_retry_backoff` retry loop; the main
     turn-state snapshot RMW already goes through `cas_update`. Migration
     tracked as follow-up #5274 (runner-lease CAS consolidation).
   - `ironclaw_threads::filesystem_service` drives `write_new_message`,
     `reserve_sequence_via_thread_record` (the legacy fallback for backends
     without native sequence reservation; `reserve_sequence` itself is now
     row-native), `apply_message_update`, `append_capability_display_preview`,
     `create_summary_artifact`, and the
     `message_sequence_index.rs`/`message_lookup_index.rs` writers through a
     local `put_with_cas` retry loop (only `ensure_thread` was migrated onto
     `cas_update`). These loops are already lock-free (no per-path mutex),
     so they are not the convoy hazard `cas_update` was introduced to fix;
     migration to `cas_update`'s fail-closed semantics is a deferred
     follow-up tracked as a sibling to #5274.
   - `ironclaw_conversations::filesystem_store::save_state`,
     `ironclaw_reborn::local_trigger_access::filesystem::deactivate_stale_record`
     (via `put_record`), and `ironclaw_product_workflow_storage::filesystem_ledger`
     (`begin_or_replay` / `settle` / `release` / `try_acquire_prune_lease`)
     are further pre-existing examples of the same lock-free retry-loop
     pattern, pending the same migration.

   See `docs/plans/2026-06-25-cas-migration.md` for the migration tracker.
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
