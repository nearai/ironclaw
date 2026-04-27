# Reborn Filesystem Contract

**Status:** Draft implementation contract
**Date:** 2026-04-24
**Depends on:** `docs/reborn/contracts/host-api.md`, `crates/ironclaw_host_api`

---

## 1. Purpose

`ironclaw_filesystem` is the durable path and mount service for IronClaw Reborn.

It turns host API path contracts into actual storage operations while preserving the core containment invariant:

```text
runtime code sees ScopedPath
host policy reasons over MountView
filesystem resolves to VirtualPath
backend code alone touches HostPath
```

The filesystem is not an agent memory system, search index, workflow engine, database abstraction, or policy engine. It is the service that safely resolves scoped paths, checks mount permissions, applies backend containment, and performs read/write/list/stat operations against mounted stores.

---

## 2. Authority model

Filesystem authority is derived from `ExecutionContext.mounts` and the requested action.

```text
ExecutionContext
  -> MountView
  -> ScopedPath
  -> permission check
  -> VirtualPath
  -> backend mount
  -> HostPath/internal backend key
```

Rules:

- runtime code never receives `HostPath`
- service APIs for untrusted callers accept `ScopedPath`, not raw strings
- root/admin APIs may accept `VirtualPath`, but only trusted host services should receive them
- missing mount = deny
- missing permission = deny
- invalid path = deny
- ambiguous backend mount = deny
- symlink/backend escape = deny
- backend failures must not reveal raw host paths in user-visible errors

---

## 3. Public service split

### 3.1 `RootFilesystem`

Trusted host-service interface. It operates on canonical `VirtualPath` values.

Use for:

- bootstrapping roots
- extension discovery under `/system/extensions`
- project/user/memory namespace setup
- internal audit/history writes
- tests that need direct virtual namespace setup

Do not expose `RootFilesystem` to WASM modules, script runner jobs, MCP tools, or third-party extensions.

### 3.2 `ScopedFilesystem`

Invocation-scoped interface. It operates on `ScopedPath` values and carries an `ExecutionContext` or an already-validated `MountView`.

Use for:

- WASM host imports
- script runner filesystem mediation
- MCP adapter file access
- first-party extension host API calls
- agent/tool work that must respect mount authority

`ScopedFilesystem` must resolve through `MountView` on every operation. It must not cache a broader authority than the context grants.

---

## 4. Namespace roots

V1 canonical virtual areas/roots mirror the storage placement map:

```text
/engine
/system/settings
/system/extensions
/system/skills
/users
/projects
/memory
/artifacts
/tmp
/secrets
/events
```

Recommended meaning:

| Root | Purpose |
|---|---|
| `/engine` | host-owned engine config, schemas, migrations, runtime metadata, and not-indexed engine blobs |
| `/system/settings` | settings repository projections/import-export surfaces |
| `/system/extensions` | installed extension packages and extension-local config/state/cache roots |
| `/system/skills` | skill package/registry projections/import-export surfaces |
| `/users` | user-owned durable profile/config areas |
| `/projects` | project workspaces, missions, thread state, and project-local config |
| `/memory` | durable memory namespace, initially file-like even if backed by another store |
| `/artifacts` | artifact/object/local backend for process and export outputs |
| `/tmp` | ephemeral runtime temp backend |
| `/secrets` | typed encrypted secret repository projection/reference surface only; not generic secret material listing |
| `/events` | durable event/audit append-log export/projection area |

Extension-visible aliases should be scoped aliases such as:

```text
/workspace
/project
/memory
/extension/config
/extension/state
/extension/cache
/tmp
/artifacts
```

Aliases are resolved by `MountView`; a same-named canonical root such as `/tmp` or `/artifacts` does not make that alias visible without an explicit mount grant.

---

## 5. Path types

### 5.1 `VirtualPath`

Canonical durable namespace path, validated by `ironclaw_host_api`.

Examples:

```text
/projects/project1/src/lib.rs
/system/extensions/github/config/settings.toml
/memory/users/user1/facts.md
```

### 5.2 `ScopedPath`

Runtime-visible path exposed through a mount alias.

Examples:

```text
/workspace/src/lib.rs
/project/missions/nightly.toml
/memory/facts.md
/extension/state/db.json
/tmp/run/output.json
/artifacts/patch.diff
```

### 5.3 `HostPath`

Backend-local physical path. It is intentionally not serializable and must not appear in audit/user-visible output.

---

## 6. Resolution contract

Resolution is two-step:

```text
ScopedPath + MountView -> VirtualPath
VirtualPath + BackendMountTable -> backend-local path/key
```

Rules:

1. Use longest alias match for `ScopedPath -> VirtualPath`.
2. Alias match must be exact or segment-boundary prefixed.
3. Path normalization rejects `..`, NUL/control characters, URLs, and raw host paths before backend resolution.
4. `VirtualPath` must begin with a known root.
5. Backend mount selection must use longest virtual mount prefix.
6. If two backend mounts match with the same prefix length, fail closed.
7. Final local filesystem canonicalization must remain inside the mounted backend root.
8. Symlinks may be read or traversed only if final canonical target remains inside the backend root.
9. Symlink writes that would create or follow an escape outside the root are denied.
10. Returned errors must identify virtual/scoped paths, not raw host paths.

---

## 7. Permissions

Use `MountPermissions` from `ironclaw_host_api`:

```rust
pub struct MountPermissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub list: bool,
    pub execute: bool,
}
```

Operation requirements:

| Operation | Required permission |
|---|---|
| `read_file` | `read` |
| `write_file` | `write` |
| `append_file` | `write` |
| `list_dir` | `list` |
| `stat` | `read` or `list` |
| `delete` | `delete` |
| `create_dir_all` | `write` |
| executable/script handoff | `execute` plus runtime-specific approval |

`write` does not imply `delete`. `read` does not imply `list` unless the mount grant explicitly carries both.

---

## 8. `/project` vs `/workspace`

Use both aliases deliberately:

- `/project` should point at the canonical project root when the invocation is allowed to reason about the whole project.
- `/workspace` should point at the working subset for the current task, branch, or sandbox.

In many local cases both aliases may target the same `VirtualPath`, but the distinction allows future task-scoped worktrees, sparse checkouts, review-only mounts, and sandbox overlays without changing extension-visible paths.

---

## 9. `/memory` resolution

`/memory` is a scoped alias backed by the canonical `/memory` virtual root.

V1 should keep memory file-like:

```text
/memory/users/<user>/...
/memory/projects/<project>/...
/memory/tenants/<tenant>/...
```

Search, embeddings, and indexing belong in a separate service layered on top of filesystem reads/writes. The filesystem may expose memory paths but must not become the semantic memory engine.

Resolution priority is mount-based, not magical:

1. if `MountView` contains `/memory`, use that target
2. otherwise the path is unavailable to the invocation
3. do not fall back to global memory automatically

---

## 10. `/tmp` lifecycle

`/tmp` is invocation- or process-scoped scratch space.

Rules:

- not durable by default
- mounted only when granted
- cleaned at invocation/process teardown unless explicitly retained for debugging
- counted against disk/output resource limits
- not indexed as memory
- not used for audit as the source of truth

The canonical virtual target may be under a host-managed scratch root such as:

```text
/engine/tmp/invocations/<invocation_id>
```

but runtimes should only see `/tmp`.

---

## 11. Artifacts and writeback

Script runner and sandboxed native CLI work should prefer artifact export over broad writable host mounts.

Recommended pattern:

```text
sandbox writes /artifacts/patch.diff or /artifacts/result.json
filesystem validates artifact path and size
host applies approved changes to /project or /workspace
host writes audit envelope with artifact hashes
```

Rules:

- artifact writes require `write` permission on `/artifacts`
- artifact export must enforce size/count limits
- host apply/writeback is a separate action requiring authorization
- script runner Docker mounts should not receive raw broad writable host paths by default

---

## 12. Backend mount table and catalog

`CompositeRootFilesystem` owns the trusted backend mount table from `VirtualPath` prefix to backend implementation. Each mount carries a `MountDescriptor` so trusted host services can answer where a path lives without probing every backend.

V1 backend types:

```text
LocalFilesystem
PostgresRootFilesystem   // feature = "postgres"
LibSqlRootFilesystem     // feature = "libsql"
Memory/test backends as needed
```

The PostgreSQL/libSQL backends store file contents by canonical `VirtualPath` in `root_filesystem_entries`; directories are inferred from path prefixes. They are database-backed `RootFilesystem` implementations for generic file-shaped content, not a mandate that every durable service becomes files.

Catalog metadata distinguishes file-shaped content from structured records and derived indexes:

```rust
pub struct MountDescriptor {
    pub virtual_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

pub struct PathPlacement {
    pub path: VirtualPath,
    pub matched_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}
```

Backend mount rules:

- mount target must be a valid `VirtualPath`
- exact duplicate mount roots fail closed
- overlapping mount roots are allowed only when longest-prefix routing is unambiguous
- longest virtual prefix wins for both catalog lookup and filesystem operations
- backend-local path joins must remain contained
- backend mount registration is a trusted host operation
- catalog lookup does not grant runtime authority; untrusted callers still need `ScopedFilesystem` plus `MountView`

Future backend types:

```text
ObjectStoreBackend
RemoteFilesystemBackend
```

Memory-specific backend adapters are owned outside this crate. The first Reborn memory seams are `ironclaw_memory::MemoryDocumentFilesystem` for the built-in repository path and `ironclaw_memory::MemoryBackendFilesystemAdapter` for plugin backends that declare file-document capability. PostgreSQL/libSQL adapters port/adapt the current workspace table family (`memory_documents`, `memory_chunks`, libSQL `memory_chunks_fts`, and `memory_document_versions`); metadata inheritance, skip flags, schema validation, embedding-provider integration, embedded chunk writes, FTS search, and rank-fused hybrid search are memory service/indexer responsibilities already represented in `ironclaw_memory`, not in the generic filesystem crate.

---

## 13. Error contract

Use a filesystem-specific service error that can wrap `HostApiError` but does not expose raw host internals.

Minimum variants:

```rust
pub enum FilesystemError {
    Contract(HostApiError),
    PermissionDenied { path: ScopedPath, operation: FilesystemOperation },
    MountNotFound { path: VirtualPath },
    PathOutsideMount { path: VirtualPath },
    SymlinkEscape { path: VirtualPath },
    MountConflict { path: VirtualPath },
    Backend { path: VirtualPath, operation: FilesystemOperation, reason: String },
}
```

Backend errors may keep raw errors for logs, but public/display errors should use scoped or virtual paths.

---

## 14. Initial Rust API sketch

```rust
#[async_trait]
pub trait RootFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError>;
    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError>;
    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError>;
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;
    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError>;
    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError>;
    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError>;
}

pub trait FilesystemCatalog {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError>;
    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError>;
}

pub struct CompositeRootFilesystem;

impl RootFilesystem for CompositeRootFilesystem { /* delegates by longest virtual prefix */ }
impl FilesystemCatalog for CompositeRootFilesystem { /* reports mount placement */ }

pub struct ScopedFilesystem<F> {
    root: F,
    mounts: MountView,
}

impl<F: RootFilesystem> ScopedFilesystem<F> {
    async fn read_file(&self, path: &ScopedPath) -> Result<Vec<u8>, FilesystemError>;
    async fn write_file(&self, path: &ScopedPath, bytes: &[u8]) -> Result<(), FilesystemError>;
    async fn append_file(&self, path: &ScopedPath, bytes: &[u8]) -> Result<(), FilesystemError>;
    async fn list_dir(&self, path: &ScopedPath) -> Result<Vec<DirEntry>, FilesystemError>;
    async fn stat(&self, path: &ScopedPath) -> Result<FileStat, FilesystemError>;
    async fn delete(&self, path: &ScopedPath) -> Result<(), FilesystemError>;
    async fn create_dir_all(&self, path: &ScopedPath) -> Result<(), FilesystemError>;
}
```

The implementation may start synchronous if the V1 local backend is synchronous, but the public trait should not block future async/remote backends.

---

## 15. Minimum TDD coverage

Add tests through the caller-facing filesystem APIs, not only helper functions:

- scoped read resolves through mount view and reads expected bytes
- read denied when mount lacks `read`
- write denied on read-only mount
- append denied on read-only mount
- list denied when mount lacks `list`
- longest backend virtual mount wins
- unknown alias fails closed
- path traversal in scoped path is rejected before backend access
- local backend denies symlink escape
- local backend does not leak raw host path in display error
- `CompositeRootFilesystem` routes operations by longest virtual mount prefix
- `FilesystemCatalog::describe_path` reports matched root, backend identity, content kind, and index policy
- exact duplicate composite mount roots fail closed
- catalog mount listing is stable for diagnostics
- PostgreSQL/libSQL backends implement `RootFilesystem` without depending on product/runtime/workflow crates
- libSQL backend reads, writes, stats, overwrites, lists direct children, infers directories, and returns virtual-path-only missing-file errors
- `/tmp` mount can be created per invocation and cleaned up
- `/artifacts` writes are captured under approved virtual path only

---

## 16. Non-goals

Do not add in `ironclaw_filesystem`:

- auth policy evaluation
- resource reservation
- search, indexing, or embeddings
- semantic memory APIs
- mission/thread orchestration
- script runner execution
- Docker/microVM/container logic
- extension manifest parsing
- network access
- secret material storage

Those are separate services using filesystem contracts.


---

## Contract freeze addendum — V1 filesystem API and placement (2026-04-25)

The V1 `RootFilesystem` contract includes exactly these operations:

```rust
read_file
write_file
append_file
list_dir
stat
delete
create_dir_all
```

`ScopedFilesystem` must expose the same operation set and enforce `MountPermissions` for each operation:

| Operation | Permission |
| --- | --- |
| `read_file` | `read` |
| `write_file` | `write` |
| `append_file` | `write` |
| `list_dir` | `list` |
| `stat` | `read` or `list` |
| `delete` | `delete` |
| `create_dir_all` | `write` |

Deferred from V1:

```text
compare-and-swap writes
rename/copy
streaming/range reads
checksums/content-addressed storage
persistent mount registry
```

`append_file` is included in V1 because event/audit JSONL sinks need append-style writes without read-modify-write truncation risk. It appends bytes at a path; it is not a compare-and-swap, lock, streaming, or multi-record transaction primitive.

The source-of-truth namespace map is frozen in [`storage-placement.md`](storage-placement.md). This filesystem crate remains generic: it may route `/memory`, `/system`, `/engine`, `/artifacts`, `/tmp`, `/secrets`, `/events`, `/users`, and `/projects` mounts, but it must not encode memory metadata/search semantics, secret repository semantics, approval/process schemas, or event projection logic.
