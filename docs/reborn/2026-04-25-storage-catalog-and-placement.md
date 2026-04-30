# Reborn Storage Catalog and Placement Plan

**Status:** Implementation planning note
**Date:** 2026-04-25
**Related contracts:** `contracts/filesystem.md`, `contracts/secrets.md`, `contracts/processes.md`, `contracts/events-projections.md`

---

## 1. Decision

Reborn should expose one virtual filesystem/mount surface to callers, but it should not force every durable service through byte-oriented filesystem APIs.

Use this split:

```text
File-shaped content and virtual path authority
  -> RootFilesystem / ScopedFilesystem / CompositeRootFilesystem

Structured, query-heavy, control-plane, or security-sensitive records
  -> typed repositories owned by the service domain

Search, chunks, embeddings, projections, and indexes
  -> derived state owned by the memory/search/projection service
```

The filesystem catalog bridges these worlds. It lets trusted host services answer where a virtual path lives and what kind of content it represents without making filesystem byte APIs responsible for secrets, approvals, processes, events, or semantic search.

---

## 2. Why this matches the current codebase

Current IronClaw already has several storage styles:

```text
memory_documents + memory_chunks
  DB-backed virtual files, chunked and indexed for search

.system/settings, .system/extensions, .system/skills, .system/engine
  DB-backed workspace documents used for machine-managed state

secrets
  typed encrypted DB records with key/lease/usage semantics

jobs, conversations, routines, users, pairings, WASM tools
  typed DB records

pending-gates.json and ~/.ironclaw/.env
  local-file persistence escape hatches

project mounts
  real filesystem backend rooted at a host path
```

So the target architecture should not pretend there is one physical storage mechanism. The target is one **authority and placement model** over multiple backends.

---

## 3. Placement rules

| Area | Virtual placement | Source of truth | Access surface | Derived state owner |
|---|---|---|---|---|
| Memory identity docs | `/memory/.../SOUL.md`, `/memory/.../AGENTS.md`, `/memory/.../USER.md` | DB memory documents | filesystem + memory APIs | memory/search service |
| Memory notes | `/memory/.../notes/*.md`, `/memory/.../MEMORY.md` | DB memory documents | filesystem + memory APIs | memory/search service |
| Project files | `/projects/...` | local/object/project backend | filesystem | optional project indexer |
| Artifacts | `/engine/tmp/.../artifacts` or `/projects/.../artifacts` | local/object/artifact backend | filesystem | artifact/result service |
| Extensions packages/config files | `/system/extensions/...` | filesystem/object/DB file backend | filesystem + extension service | extension service |
| Secrets | no general file mount for material | typed secret repository | secrets service | secrets service |
| Approval leases/gates | no general file mount | typed approval/run-state repository | approvals/run-state services | audit/projection service |
| Process lifecycle/results | no general file mount for control records | typed process repositories | process service | event/projection service |
| Events/audit logs | append/projection repositories; optional file-shaped exports | typed event sink/projection store | event service | projection service |
| Raw DB search/vector indexes | not mounted | DB indexes/tables | memory/search service | memory/search service |

Rule of thumb:

```text
If callers reasonably think in paths and bytes/text, mount it.
If callers need lifecycle, locking, transactions, leases, query predicates,
redaction, or encryption semantics, keep a typed repository.
```

---

## 4. Catalog model

`ironclaw_filesystem` owns a trusted catalog interface:

```rust
pub trait FilesystemCatalog {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError>;
    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError>;
}
```

The catalog describes mount placement only. It does not grant access by itself. Runtime callers still need `ScopedFilesystem` and `MountView` authority.

Minimum descriptor shape:

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

This makes placement explicit enough for admin diagnostics, migration tools, docs, and host-service wiring.

---

## 5. Composite filesystem model

`CompositeRootFilesystem` is the first implementation slice.

Responsibilities:

- register trusted backend mounts with `MountDescriptor`
- choose the longest matching virtual root for filesystem operations
- delegate `read_file`, `write_file`, `list_dir`, and `stat` to the matched backend
- expose catalog metadata for registered mounts
- fail closed for missing mounts and duplicate exact roots
- avoid product/runtime/workflow dependencies

Non-responsibilities:

- no authorization decisions beyond existing scoped mount permissions
- no memory chunking/search implementation
- no secret material repository
- no approval/process/event schema
- no DB migration for memory yet

---

## 6. Memory backend direction

The memory filesystem backend adapts the existing workspace model into Reborn instead of storing memory as opaque bytes in `root_filesystem_entries`.

Canonical path shape:

```text
/memory/tenants/{tenant_id}/users/{user_id}/projects/{project_id-or-_none}/SOUL.md
/memory/tenants/{tenant_id}/users/{user_id}/projects/{project_id-or-_none}/MEMORY.md
/memory/tenants/{tenant_id}/users/{user_id}/projects/{project_id-or-_none}/AGENTS.md
/memory/tenants/{tenant_id}/users/{user_id}/projects/{project_id-or-_none}/USER.md
/memory/tenants/{tenant_id}/users/{user_id}/projects/{project_id-or-_none}/notes/*.md
```

Implemented first seam in `ironclaw_memory`:

```rust
MemoryDocumentFilesystem
MemoryDocumentScope
MemoryDocumentPath
MemoryDocumentRepository
MemoryDocumentIndexer
InMemoryMemoryDocumentRepository
LibSqlMemoryDocumentRepository
PostgresMemoryDocumentRepository
```

`ironclaw_filesystem` remains generic. `ironclaw_memory` owns memory-specific path grammar, scope parsing, repository delegation, directory inference, and best-effort write-after-persist index hook invocation.

The PostgreSQL/libSQL repository adapters map file-shaped memory documents into the existing `memory_documents` table shape. The adapter stores scoped owner identity, including the project id or `_none` sentinel, in the `user_id` column and keeps `path` as the user-visible relative document path. This avoids reserving ordinary top-level paths such as `projects/...` inside no-project memory scopes while staying compatible with the current document table shape.

The current repository adapters intentionally touch only document rows:

```text
memory_documents
```

These remain owned by later memory service/indexer wiring:

```text
memory_chunks
memory_document_versions
embedding/search index tables
```

`RootFilesystem::read_file` and `write_file` expose file-shaped documents; the memory service/repository owns indexing, embedding, metadata inheritance, versioning, and search. Repository writes are the source of truth; index refresh failures leave derived state stale but do not roll back or report the committed write as failed.

---

## 7. Secrets direction

The filesystem-backed secrets branch is useful as a verified experiment/reference, but final production placement should prefer typed secret repositories for secret records and leases.

Reasons:

- encrypted secret records need transactional consume/update semantics
- lease state is structured, scoped, and query-heavy
- secret material must not be discoverable through a generic file listing surface
- credential mapping and usage metadata belong to the secret service boundary

A future diagnostic/export feature may emit redacted secret metadata as file-shaped reports, but that is a projection, not the source of truth.

---

## 8. Implementation sequence

1. Add `MountDescriptor`, `PathPlacement`, `BackendCapabilities`, `ContentKind`, `IndexPolicy`, and `FilesystemCatalog` to `ironclaw_filesystem`.
2. Add `CompositeRootFilesystem` with longest-prefix backend routing and catalog lookup.
3. Update `contracts/filesystem.md` and filesystem guardrails.
4. Add a memory-document backend design/implementation slice that maps `/memory/...` virtual files to memory document repositories.
5. Return to secrets/keychain/`InjectSecretOnce` only after the typed repository vs file-shaped projection split is locked.

---

## 9. Success criteria for the first slice

- callers can mount multiple backend filesystems behind one `RootFilesystem`
- catalog lookup explains which backend owns a virtual path
- overlapping roots use longest-prefix routing
- exact duplicate roots fail closed
- missing roots fail closed without backend side effects
- no new dependency edges from `ironclaw_filesystem` into product, runtime, secret, approval, process, or event crates
