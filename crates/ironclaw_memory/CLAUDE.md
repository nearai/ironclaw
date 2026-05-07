# ironclaw_memory guardrails

- Own memory document repository seams, `/memory` virtual path grammar, memory backend plugin contracts, memory-document filesystem adapters, and indexer hook boundaries.
- Depend on `ironclaw_host_api` and `ironclaw_filesystem`; do not move generic mount/catalog logic here.
- Memory backends are plugins behind host-resolved scope. They must not infer broader tenant/user/agent/project authority or bypass mount/scoped filesystem checks.
- Do not depend on the main app crate, `src/workspace`, product workflow, dispatcher, concrete runtimes, approvals, run-state, secrets, network, process, events, or extension crates.
- Keep semantic search, chunking, embeddings, and versioning behind memory-owned repository/indexer abstractions; do not put them in `ironclaw_filesystem`.
- Reborn memory is **native and isolated**. Persistence lives in dedicated `reborn_memory_*` tables with explicit `tenant_id`, `user_id`, `agent_id`, `project_id` scope columns. Do not encode Reborn scope into legacy `memory_documents.user_id` and do not introduce a `WorkspaceMemoryAdapter` or any other bridge over `src/workspace::Workspace`.
- `src/workspace/*` and `src/db/libsql/workspace.rs` are **reference material only**. Pure behavior, schema validation, FTS escaping, chunking, version-hash semantics, RRF/weighted hybrid search fusion, and `.config` inheritance tests may be ported, but `ironclaw_memory` must not depend on the main app crate or any product modules to do so.
- Legacy migration and coexistence of existing `memory_documents` rows are **explicitly deferred** to a later issue that defines the product mapping. Do not migrate or alias legacy rows from this crate.
- Every read/list/search/write/version/chunk operation must filter by the full `(tenant_id, user_id, agent_id, project_id)` tuple. Do not infer project scope from path prefixes. Document uniqueness must be `UNIQUE (tenant_id, user_id, agent_id, project_id, path)`.
- Use the empty string as the DB-only absent sentinel for `agent_id` and `project_id` (safe because `MemoryDocumentScope` rejects empty supplied IDs). Do not store `_none` in the database — `_none` is the **virtual-path** sentinel only.
- Capability declarations (`MemoryBackendCapabilities`) are enforcement inputs: unsupported file/search behavior must fail closed before backend side effects.
- Treat document writes as committed once persistence succeeds: a derived index/embedding refresh failure after persistence must not make the write report failure.
- Both libSQL and Postgres repositories must have real behavioral test coverage. Postgres compile-only coverage is not sufficient; use the established testcontainer/deadpool pattern when adding the harness.
