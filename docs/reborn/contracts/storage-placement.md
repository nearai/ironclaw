# Reborn Contract — Storage Placement

**Status:** Contract-freeze draft
**Date:** 2026-04-25
**Depends on:** [`host-api.md`](host-api.md), [`filesystem.md`](filesystem.md), [`memory.md`](memory.md), [`secrets.md`](secrets.md), [`events-projections.md`](events-projections.md)

---

## 1. Purpose

This contract freezes where durable state lives in Reborn.

The rule is hybrid:

```text
File-shaped content and virtual path authority
  -> RootFilesystem / ScopedFilesystem / CompositeRootFilesystem

Structured, query-heavy, control-plane, or security-sensitive records
  -> typed repositories owned by the service domain

Derived data such as chunks, search indexes, embeddings, and projections
  -> the owning service/indexer/projection layer
```

This prevents two failure modes:

1. forcing every durable record into ad hoc JSON files;
2. hiding file-shaped content behind unrelated typed APIs.

---

## 2. Scope model

Every durable placement must be scoped by the global Reborn scope model where applicable:

```text
tenant_id     required for hosted production state
user_id       required for user-owned state
project_id    optional, required for project-owned state
agent_id      optional first-class scope for per-agent memory/state isolation
mission_id    optional project execution scope
thread_id     optional conversation/turn scope
process_id    optional runtime/process scope
invocation_id optional effect/request scope
```

`AgentId` is first-class because current production workspace memory has optional `agent_id` partitioning. New storage contracts must either carry it or explicitly state why the domain is not agent-scoped.

---

## 3. Canonical namespace/source-of-truth map

| Virtual area | Source of truth | Access surface | Indexed? | Notes |
| --- | --- | --- | --- | --- |
| `/memory` | `ironclaw_memory` DB repositories over `memory_documents`, `memory_chunks`, `memory_document_versions` | file-shaped memory docs + memory service APIs | backend-defined full-text/vector | Memory-specific path grammar lives in `ironclaw_memory`, not filesystem. |
| `/projects` | local/object/project file backend | filesystem | optional project indexer | Project source files and user-authored project artifacts. |
| `/system/settings` | typed settings repository | typed API + optional file projection | no, unless projection says otherwise | Settings source of truth is not memory. |
| `/system/extensions` | extension package/registry repositories | extension API + filesystem package reads/projections | no semantic memory indexing | Installed packages, manifests, registry state. |
| `/system/skills` | skill package/registry repositories | skill API + optional file projection | no semantic memory indexing | Skill manifests and installed skill state. |
| `/engine/runtime` | typed run/thread/process/turn repositories, or NotIndexed `/engine` DB filesystem for file-shaped runtime blobs | typed APIs primarily | no | High-churn runtime state must not pollute memory indexes. |
| `/artifacts` | artifact/object/local backend | artifact APIs + filesystem refs | no semantic memory indexing by default | Large/binary/process output refs live here. |
| `/tmp` | ephemeral runtime temp backend | scoped filesystem | no | Process/invocation-local temporary data. |
| `/secrets` | typed encrypted secret repository | secret APIs only; optional redacted projection | no | No generic listing of secret material/source records. |
| `/events` | durable event/audit append log + projections | event/projection APIs; optional export | no | Events are append/projection records, not mutable files. |

---

## 4. Placement rules by content type

### 4.1 File-shaped user/project content

Examples:

```text
/projects/{project}/src/lib.rs
/projects/{project}/README.md
/artifacts/{process}/result.json
```

Rules:

- source of truth may be local filesystem, object store, or DB-backed file store;
- access to runtimes goes through `ScopedFilesystem` and `MountView`;
- raw host paths never appear in runtime-visible paths, errors, events, or audit;
- indexing is explicit and owned by a project/artifact indexer, not by `RootFilesystem`.

### 4.2 Memory documents

Examples:

```text
/memory/tenants/{tenant}/users/{user}/agents/{agent-or-_none}/projects/{project-or-_none}/MEMORY.md
/memory/tenants/{tenant}/users/{user}/agents/{agent-or-_none}/projects/{project-or-_none}/daily/2026-04-25.md
```

Rules:

- source of truth is the memory repository, preserving existing production table family where viable;
- memory docs are file-shaped, but memory search/chunks/versions are structured derived state;
- memory path grammar, metadata inheritance, versioning, search, prompt context, and layer rules live in `ironclaw_memory`;
- `ironclaw_filesystem` may route/mount memory backends but must not encode memory semantics.

### 4.3 Structured control-plane state

Examples:

```text
settings
extension registry
skill registry
approvals
run-state
process records
resource reservations
secret records
event/audit records
```

Rules:

- source of truth is a typed repository owned by the domain;
- optional file-shaped projections may exist for diagnostics, import/export, or admin editing;
- projections must not become the hidden source of truth unless the contract explicitly says so;
- projection writes, if allowed, validate schema and then call the typed repository.

### 4.4 High-churn runtime state

Examples from current production:

```text
engine/.runtime/**
engine/projects/**
engine/orchestrator/failures.json
engine/README.md
```

Rules:

- must not be indexed as semantic memory;
- should use typed runtime repositories when queryable;
- if file-shaped blobs are needed, mount under `/engine/runtime` or `/engine` with `IndexPolicy::NotIndexed`;
- writes should not create memory chunks, embeddings, or memory versions unless explicitly converted into knowledge.

---

## 5. Filesystem catalog requirements

Every mounted filesystem backend should expose a `MountDescriptor` containing:

```text
virtual_root
backend_id
backend_kind
storage_class
content_kind
index_policy
capabilities
```

Here `capabilities` means backend support flags. It is not the same concept as extension capability declarations.

Catalog lookup answers placement only. It does not grant authority.

Untrusted/runtime access still requires:

```text
ScopedPath -> MountView -> permission check -> VirtualPath -> backend
```

---

## 6. Backend support policy

Backend capability fields are support declarations, not extension capability declarations and not authority grants.

The terminology is overloaded today because some types already use names such as `BackendCapabilities` and `MemoryBackendCapabilities`. In this storage contract, those fields mean:

```text
what this backend can safely perform after the host has already authorized scope and selected the backend
```

They do not mean:

```text
caller-visible extension action
approval/lease authority
permission to bypass ScopedFilesystem or MountView checks
```

Backend support declarations are enforcement inputs, not documentation only. Unsupported behavior fails before backend side effects.

Examples:

- if `delete = false`, delete fails before backend side effects;
- if `indexed = false`, callers must not assume search visibility;
- if `embedded = false`, vector search must fail closed or omit vector results;
- if a memory backend sets `file_documents = false`, `/memory` file operations fail closed before plugin invocation.

A future implementation cleanup may rename these backend fields/types to `*Support` for clarity, but the frozen contract already distinguishes support declarations from extension capabilities.

---

## 7. Engineer task implications

Before implementing a persistence task, engineers must identify:

1. virtual area/prefix;
2. source-of-truth repository/backend;
3. scope fields, including `AgentId` if relevant;
4. whether filesystem access is source-of-truth or projection;
5. indexing policy;
6. delete/versioning behavior;
7. PostgreSQL/libSQL parity requirement;
8. migration/backfill impact.

If the answer is not in this document or the owning domain contract, the task is not ready for implementation.

---

## 8. Acceptance tests for placement changes

Any new storage placement must include:

- tenant/user/project/agent isolation test when scoped;
- source-of-truth test proving writes go to the intended repository/backend;
- projection test if file views are exposed;
- no-indexing test for control-plane/runtime state;
- redaction/no-host-path test for errors/events;
- PostgreSQL/libSQL parity test if the backend is production-persistent.
