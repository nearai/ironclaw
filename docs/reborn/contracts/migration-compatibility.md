# Reborn Contract — Migration and Compatibility

**Status:** Contract-freeze draft
**Date:** 2026-04-25
**Depends on:** [`storage-placement.md`](storage-placement.md), [`memory.md`](memory.md), [`settings-config.md`](settings-config.md), [`secrets.md`](secrets.md)

---

## 1. Purpose

Reborn must preserve current production data where viable.

Frozen migration principle:

```text
Reuse existing schemas where viable, bridge only when necessary.
```

This contract tells engineers when to adapt existing tables and when a new schema is justified.

---

## 2. General rules

1. Do not replace a working production table with a new Reborn table unless the existing shape cannot satisfy the frozen contract.
2. Prefer repository/adapters around existing schemas.
3. Add migrations only for missing columns/indexes/constraints that are contractually required.
4. PostgreSQL and libSQL compatibility must be considered together.
5. Released migrations are immutable; changes require new migrations.
6. Rollback/compatibility risks must be documented in the implementing PR.
7. Backfills must be idempotent.
8. Multi-tenant scope isolation must be tested after every schema or mapping change.

---

## 3. Memory schema compatibility

Existing production family to preserve where viable:

```text
memory_documents
memory_chunks
memory_chunks_fts        # libSQL
memory_document_versions
```

Required Reborn mapping:

```text
tenant_id + user_id -> scoped owner key in user_id, unless future migration adds tenant_id directly
agent_id            -> existing agent_id column when present
project_id          -> project path prefix or future explicit column
relative path       -> path
content             -> content
metadata            -> metadata JSON/JSONB
chunks              -> memory_chunks
versions            -> memory_document_versions
```

Compatibility requirements:

- existing rows with `agent_id = NULL` remain visible under absent `AgentId`;
- existing agent-scoped rows map to Reborn `AgentId`;
- current `sha256:{hex}` version hash format remains valid;
- libSQL FTS trigger behavior remains valid;
- pgvector/libSQL vector embeddings remain readable where dimensions match;
- project path prefix mapping must keep existing documents discoverable.

If a future migration adds explicit `tenant_id`, `project_id`, or `agent_id` string columns, it must dual-read or backfill from existing rows before switching reads.

---

## 4. Root filesystem compatibility

`root_filesystem_entries` is valid for generic DB-backed file content.

Use it for:

```text
file-shaped state that does not need domain-specific query semantics
small JSON/file projections where DB file storage is appropriate
```

Do not use it as source of truth for:

```text
memory documents with search/index/version requirements
secrets production records
settings source-of-truth records
events/audit append logs
process/control-plane records needing typed queries
```

---

## 5. Secrets compatibility

Production source of truth is a typed encrypted secret repository.

Existing secrets data should be ported/adapted from current tested implementation:

```text
src/secrets/crypto.rs
src/secrets/keychain.rs
src/secrets/store.rs
src/secrets/types.rs
```

Existing schema lineage:

```sql
secrets(
  id,
  user_id,
  name,
  encrypted_value,
  key_salt,
  provider,
  expires_at,
  last_used_at,
  usage_count,
  created_at,
  updated_at
)
```

Rules:

- preserve encrypted material compatibility where possible;
- do not expose secret records through generic file listing;
- filesystem-backed encrypted JSON remains a reference/projection mode, not production default;
- migration must preserve usage metadata;
- master-key/keychain resolution must be documented and tested before production injection.

---

## 6. Settings/config compatibility

Production stores some system state as workspace docs:

```text
.system/settings/**
.system/extensions/**
.system/skills/**
.system/engine/**
```

Reborn production source of truth is typed repositories with optional file projections.

Migration strategy:

1. read existing `.system/...` workspace docs;
2. validate against owning schema;
3. import into typed repositories;
4. keep file projections for compatibility and diagnostics;
5. mark unsupported/malformed documents with actionable migration diagnostics.

Projection write-back is allowed only if the projection contract says so and must call the typed repository.

---

## 7. Engine/runtime state compatibility

High-churn engine runtime blobs from production must not be indexed as memory.

Examples:

```text
engine/.runtime/**
engine/projects/**
engine/orchestrator/failures.json
engine/README.md
```

Migration strategy:

- move queryable runtime state to typed repositories;
- move file-shaped runtime blobs to `/engine/runtime` or `/engine` with `IndexPolicy::NotIndexed`;
- delete stale memory chunks for those paths during migration/backfill where safe;
- do not create semantic memory versions for high-churn runtime blobs unless explicitly promoted to knowledge.

---

## 8. Events/audit compatibility

Events/projections require a durable append log with replay cursors.

Migration strategy:

- current JSONL/in-memory sinks may be adapters or development backends;
- production event log must support cursor replay and scoped retention;
- audit records remain redacted and are not replaced by UI event projections;
- existing logs may be imported best-effort if schema can be validated.

---

## 9. Cutover model

Reborn should ship to users as a coherent composition path, not as partially exposed islands. Internal Reborn crates and adapters may land incrementally, but production exposure should be guarded by a feature flag, parallel binary, or equivalent deployment switch until the caller-level integration spine is complete.

Cutover requirements:

- one config-driven production composition root for `HostRuntimeServices` and adjacent shipped reference/userland services;
- caller-level tests across `CapabilityHost -> Dispatcher -> Adapter -> Process/Event/Memory` paths;
- explicit migration/backfill path for reused legacy schemas;
- rollback notes for any bridge that can affect production state;
- no accidental dual source of truth between legacy `src/` services and Reborn repositories.

Legacy `src/` changes remain allowed for security, urgent customer fixes, and explicitly approved compatibility work. New non-trivial product work should prefer Reborn crates when practical, but a hard `src/` feature freeze is a separate policy decision rather than part of this contract update.

---

## 10. Migration acceptance tests

Every migration/compatibility task must include:

- old-shape fixture read test;
- new-shape write test;
- idempotent backfill test;
- tenant/user/project/agent isolation test;
- PostgreSQL/libSQL parity or explicit backend exclusion;
- rollback/failure-mode note;
- no-leak assertion for secrets/events/errors where relevant;
- `git diff --check` and targeted DB migration checks.

---

## 11. When new schemas are allowed

A new schema is justified when the existing schema cannot support one of:

- required scope isolation, especially `tenant_id`/`AgentId`;
- required query shape without unsafe scans;
- required transactionality;
- required redaction/security invariant;
- required event cursor ordering;
- required lifecycle state machine;
- required retention/deletion semantics.

The PR must state why an adapter over existing schema is insufficient.
