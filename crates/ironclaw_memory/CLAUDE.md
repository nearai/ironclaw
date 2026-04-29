# ironclaw_memory guardrails

- Own memory/workspace document repository seams, `/memory` virtual path grammar, memory backend plugin contracts, memory-document filesystem adapters, and indexer hook boundaries.
- Depend on `ironclaw_host_api` and `ironclaw_filesystem`; do not move generic mount/catalog logic here.
- Memory backends are plugins behind host-resolved scope. They must not infer broader tenant/user/project authority or bypass mount/scoped filesystem checks.
- Do not depend on product workflow, dispatcher, concrete runtimes, approvals, run-state, secrets, network, process, events, or extension crates.
- Keep semantic search, chunking, embeddings, and versioning behind memory-owned repository/indexer abstractions; do not put them in `ironclaw_filesystem`.
- Reuse-first rule: port/adapt the current working workspace implementation (`src/workspace/*`, `src/db/libsql/workspace.rs`, migrations) rather than inventing parallel memory DB/chunk/version/search behavior.
- PostgreSQL/libSQL repository adapters should preserve the existing `memory_documents`, `memory_chunks`, `memory_chunks_fts`, and `memory_document_versions` table shapes. Chunk/search/version updates remain explicit memory-owned indexer/service work.
- Metadata/search behavior should stay reuse-first: preserve current `DocumentMetadata`, `.config` inheritance, `skip_indexing`, `skip_versioning`, schema validation, FTS query escaping, embedding vector storage, RRF/weighted hybrid search fusion, and version hash semantics unless deliberately changed with tests/docs.
- Capability declarations (`MemoryBackendCapabilities`) are enforcement inputs: unsupported file/search behavior must fail closed before backend side effects.
- Preserve tenant/user/project scope on every path parse and repository operation.
- Treat `_none` as the virtual path sentinel for absent project ids; never store it as a real project id.
