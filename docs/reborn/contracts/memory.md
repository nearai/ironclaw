# Reborn Contract — Memory and Workspace Services

**Status:** Contract-freeze draft
**Date:** 2026-04-25
**Target crate:** `crates/ironclaw_memory` plus service composition crates
**Depends on:** [`host-api.md`](host-api.md), [`filesystem.md`](filesystem.md), [`storage-placement.md`](storage-placement.md), [`network.md`](network.md), [`secrets.md`](secrets.md), [`events-projections.md`](events-projections.md)

---

## 1. Purpose

`ironclaw_memory` owns durable memory/workspace document semantics for Reborn.

It is responsible for:

- memory-specific virtual path grammar;
- repository seams for DB-backed memory documents;
- memory backend plugin contracts;
- metadata and `.config` inheritance;
- chunking, indexing, embeddings, and search;
- memory service facades over shared backends;
- prompt-context assembly policy;
- seeding/bootstrap/profile sync policy;
- memory layers and multi-scope reads.

It is not responsible for:

- generic filesystem mount/catalog logic;
- authorization/grant evaluation;
- runtime dispatch;
- secret storage/injection;
- network policy enforcement internals;
- event transport protocols.

---

## 2. Scope model

Memory scope includes:

```text
tenant_id  required
user_id    required
agent_id   optional first-class scope
project_id optional
```

`AgentId` is preserved for production parity with the existing `memory_documents.agent_id` partition.

The canonical Reborn memory path should include agent scope:

```text
/memory/tenants/{tenant}/users/{user}/agents/{agent-or-_none}/projects/{project-or-_none}/{relative/path}
```

The current implementation path without `agents/{agent}` is transitional. Contract-finalizing implementation work must add `AgentId` before declaring memory service parity complete.

Rules:

- `_none` is reserved and means an absent optional scope;
- backends receive a host-resolved `MemoryContext` and must not widen authority;
- memory plugins do not decide tenant/user/project/agent authority;
- all persistence, search, events, resources, and audit records must carry the relevant memory scope.

---

## 3. Source-of-truth storage

Built-in memory source of truth reuses existing production schema where viable:

```text
memory_documents
memory_chunks
memory_chunks_fts        # libSQL FTS5
memory_document_versions
```

Mapping:

```text
MemoryScope
  tenant_id + user_id + optional agent_id + optional project_id

Existing DB shape
  user_id  = scoped owner key, e.g. tenant:{tenant}:user:{user}
  agent_id = optional AgentId when present
  path     = relative path, with project prefix where needed
```

Project mapping may continue to use:

```text
projects/{project_id}/{relative_path}
```

until a migration changes the physical schema. Any migration must preserve read compatibility for existing rows.

---

## 4. Low-level backend contracts

The low-level public contracts are:

```rust
MemoryBackend
MemoryBackendCapabilities
MemoryContext
MemorySearchRequest
MemorySearchResult
MemoryBackendFilesystemAdapter
RepositoryMemoryBackend
MemoryDocumentScope
MemoryDocumentPath
MemoryDocumentRepository
MemoryDocumentIndexRepository
MemoryDocumentIndexer
EmbeddingProvider
```

`MemoryBackendCapabilities` are backend support declarations, not extension capability declarations and not authority grants.

The distinction is intentional:

- extension capability declarations describe caller-visible actions such as `github.search_issues` and feed authorization, approval, lease, and runtime dispatch;
- memory backend support declarations describe what an already-selected backend can safely perform;
- backend support declarations never grant access by themselves. Host scope, authorization, leases, and filesystem mount authority are checked first.

Backend support declarations are still enforcement inputs. Unsupported behavior fails before backend side effects.

Examples:

- `file_documents = false` means file operations fail closed;
- `full_text_search = false` means full-text search fails closed when requested;
- `vector_search = false` means vector search fails closed when explicitly requested;
- `embeddings = false` means host/provider embedding should not be assumed;
- plugin errors must be sanitized and scoped.

The current type name is `MemoryBackendCapabilities` for implementation continuity. A future cleanup may rename it to `MemoryBackendSupport` or similar, but the contract meaning is support declaration, not extension capability authority.

---

## 5. Service split

Reborn memory should expose focused services over the shared backend, not one monolithic production `Workspace` clone.

### 5.1 `MemoryDocumentService`

Owns user-facing document operations:

```text
read
read_primary
exists
write
append
append_memory
append_daily_log
delete
list
list_all
patch
```

Rules:

- writes target the primary scope unless explicitly routed through a layer;
- append and patch validate the final content, not only the delta;
- write/append/patch version previous content before mutation according to version policy;
- write/append/patch reindex after mutation according to metadata policy;
- system-prompt-file write safety is delegated to kernel-mediated prompt safety policy hooks.

### 5.2 `MemorySearchService`

Owns:

```text
single-scope search
multi-scope search
full-text/vector/hybrid fusion
search config defaults
identity-document filtering
embedding query generation
```

Rules:

- full-text + vector rank fusion uses RRF by default;
- weighted rank fusion remains supported;
- secondary-scope identity/system-prompt documents are excluded from search results;
- Postgres should implement unified multi-scope search when practical;
- libSQL may use per-scope merge but must document score comparability limits.

### 5.3 `MemoryVersionService`

Owns:

```text
save version
get version
list versions
latest version lookup
prune versions
patch version behavior
```

Rules:

- empty previous content is not versioned;
- duplicate latest content hash is not versioned again;
- `skip_versioning` suppresses version creation;
- version attribution records the actor scope/user, not necessarily the target layer scope;
- caller-level APIs must verify ownership before using bare document IDs.

### 5.4 `MemoryLayerService`

Owns named read/write layers:

```text
private
shared/team
custom named layers
```

Rules:

- layer scopes must be namespaced and cannot collide with raw user IDs;
- layers declare readable/writable flags;
- writes to read-only layers fail closed;
- optional privacy classifiers may redirect sensitive shared-layer writes to private layers;
- redirects must be visible in the returned write result;
- layer metadata resolution happens in the target layer scope.

### 5.5 Prompt safety policy and reference prompt assembly

Prompt-injection write safety is kernel-mediated policy because these files can affect future execution context. Prompt assembly is loop/userland strategy over authorized memory reads, not a single kernel-owned behavior.

A reference prompt assembler may read prompt files such as:

```text
BOOTSTRAP.md
AGENTS.md
SOUL.md
USER.md
IDENTITY.md
SYSTEM.md
MEMORY.md
TOOLS.md
HEARTBEAT.md
context/profile.json
context/assistant-directives.md
```

Kernel-mediated safety rules:

- identity files are primary-scope only;
- admin `SYSTEM.md` is admin-scope only and may be cached;
- `MEMORY.md` and daily logs may read configured secondary scopes only through explicit read-scope policy;
- group chat contexts exclude personal memory/profile/directives unless explicit policy allows them;
- writes to prompt-injected files are scanned by the safety sanitizer;
- empty writes used to clear bootstrap/profile state may bypass injection scanning only by explicit policy;
- raw prompt-injection details should not leak into unrelated events.

Loop/userland assembly rules:

- active loops may choose prompt order, summarization, model-specific formatting, and inclusion heuristics over authorized reads;
- custom loops cannot bypass primary-scope identity filtering, group-chat privacy filtering, write-safety checks, or event/audit redaction;
- reference loop prompt assemblers are replaceable behavior, not memory backend source of truth.

### 5.6 `MemorySeedService`

Owns initial and upgrade seeding:

```text
README.md
MEMORY.md
IDENTITY.md
SOUL.md
AGENTS.md
USER.md
HEARTBEAT.md
TOOLS.md
.system/gateway/README.md
daily/.config
conversations/.config
.system/gateway/.config
BOOTSTRAP.md
```

Rules:

- seeds never overwrite user-authored content;
- fresh-workspace detection uses primary-scope identity docs;
- `BOOTSTRAP.md` is seeded only for truly fresh workspaces without an existing populated profile;
- import-from-directory is non-recursive unless explicitly extended and never overwrites existing docs.

### 5.7 `MemoryProfileService`

Owns:

```text
context/profile.json
USER.md profile section merge
context/assistant-directives.md
HEARTBEAT.md generation
profile prompt context inclusion
```

Rules:

- generated profile content uses delimiters and preserves user-authored sections;
- old auto-generated profile sections may be migrated;
- profile context is excluded from group chat;
- profile schema validation is explicit.

---

## 6. Metadata contract

`DocumentMetadata` supports:

```text
skip_indexing: Option<bool>
skip_versioning: Option<bool>
hygiene: Option<HygieneMetadata>
schema: Option<serde_json::Value>
extra fields preserved
```

Rules:

- metadata merges shallowly;
- document metadata overrides nearest ancestor `.config` metadata;
- `.config` lookup walks parent directories from nearest to root;
- `schema: null` means no schema;
- schema validation happens before persistence;
- schema validation errors are stable/sanitized and tied to virtual path, not host path.

---

## 7. Indexing and embeddings

Indexing contract:

```text
write document
  -> resolve metadata
  -> validate schema
  -> persist document
  -> if skip_indexing delete chunks
  -> else chunk document
  -> optionally embed chunks
  -> replace chunks if document content hash still matches
```

Rules:

- chunking preserves current production word-overlap behavior;
- chunk replacement is transactional per document;
- stale index replacement is skipped if document content changed during embedding;
- embedding provider failures during write-time indexing should not corrupt stored document content;
- production provider HTTP must go through `ironclaw_network`;
- provider credentials must be resolved through secret leases, not direct env leakage, unless the provider contract explicitly marks local no-secret operation.

Backfill contract:

```text
get_chunks_without_embeddings
update_chunk_embedding
backfill_embeddings(batch_size)
```

Backfill is required for V1 production parity but may be implemented after core write-time embedding.

---

## 8. Multi-scope read contract

A memory service has:

```text
primary scope       write target by default
secondary scopes    explicit read-only scopes
layer scopes        explicit named read/write scopes
```

Rules:

- primary scope is always first in precedence;
- secondary scopes never supply identity/system-prompt files;
- list/list_all merge and deduplicate results;
- if two scopes contain the same relative path, primary scope wins for direct reads;
- search excludes secondary identity/system-prompt docs;
- write APIs must never update a document found only through secondary read scope unless a layer write explicitly targets that layer.

---

## 9. Events and audit

Memory service events should use the durable event contract.

Minimum event classes:

```text
memory.document_written
memory.document_deleted
memory.document_indexed
memory.index_skipped
memory.search_performed
memory.layer_redirected
memory.prompt_context_built
memory.safety_rejected
```

Rules:

- events include tenant/user/project/agent scope;
- events do not include raw document contents by default;
- prompt context events do not include assembled prompts;
- sanitizer rejection events include stable reason categories, not full sensitive content;
- embedding/provider failures include sanitized provider/error kind.

---

## 10. Required acceptance tests

Delegated memory work must include relevant tests from this list:

- tenant/user/project/agent path isolation;
- legacy `agent_id` mapping compatibility;
- primary-only identity reads in multi-scope mode;
- secondary-scope identity filtering from list/search;
- layer namespace collision rejection;
- privacy redirect returns visible redirect metadata;
- `.config` inheritance and document override;
- `skip_indexing` deletes existing chunks;
- `skip_versioning` suppresses version creation;
- schema validation rejects before persistence;
- stale index replacement is skipped on concurrent content changes;
- full-text/vector/hybrid search ordering;
- provider HTTP path uses `ironclaw_network`;
- prompt-injected file sanitizer rejection through the service caller;
- PostgreSQL/libSQL parity for repository behavior.
