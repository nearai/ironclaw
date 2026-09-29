# tool-retrieval — the dense tool ranker

A semantic (embedding) ranker for `tool_search`. It implements the loop-tier
retrieval port (`ToolRetrievalProvider` / `ToolRetrievalIndex` in
`ironclaw_loop_contracts::tool_retrieval`) with an `EmbeddingProvider` from
`ironclaw_llm::embeddings`. The host-bundled BM25F ranker in
`ironclaw_loop_host` stays the default; this one is bound only when the
operator opts in.

- **Code:** crate `ironclaw_tool_retrieval` (this directory: `Cargo.toml`,
  `src/`, `tests/`). No `manifest.toml`: this is not an installable extension
  and declares no capability surface. It is a provider behind a loop-host port,
  linked by composition.
- **Layer:** `substrates`, like the memory provider packages.
- **Depends on:** `ironclaw_loop_contracts` (the port), `ironclaw_host_api`
  (ids and description provenance), `ironclaw_llm` (the embeddings port),
  `ironclaw_filesystem` (the plane its vector store persists through).
- **Bound by:** `ironclaw_composition::resolve_tool_retrieval_provider`, when
  `REBORN_TOOL_RETRIEVAL=dense` (alone) or `hybrid` (as the dense side of
  `ironclaw_loop_host::HybridToolRetrieval`) and an `[embeddings]` provider is
  configured. Either mode without an embeddings provider refuses startup.
  The runtime build binds the vector store it persists to.

## How it ranks

- **Fit.** One document per authorized tool: its name, description, and
  parameter names (with parameter descriptions only when the tool came from a
  verified catalog, as the native ranker does). Documents are bounded (4 KiB
  each, 64 parameters, bounded schema walk) and at most 2048 tools are
  accepted per fit. A fit looks each document's vector up in the memory cache,
  then (for an owner-scoped fit) in the owner's durable store, and hands only
  what is still missing to a background embedding job, which it waits for.
- **Search.** The query (trimmed, cut to 4 KiB, the largest conversation
  segment turn-start selection ranks) is embedded, and every tool is scored
  by brute-force cosine similarity. `search_many` embeds several queries in
  one request (the batch the embeddings client splits by its own batch size)
  and ranks each on its own vector, exactly as `search` would; turn-start
  selection uses it to rank every conversation segment at once.

## Where vectors come from

- **Memory cache.** Vectors are cached by the SHA-256 of the document text, so
  a catalog change re-embeds only new or changed tools. It holds at most 4096
  vectors (twice the corpus limit) and evicts the entries used by the oldest
  fit first; entries the current fit uses are never evicted. It is
  process-wide: a vector is a pure function of the document text and the
  embedding space, so a hit returns exactly what embedding the caller's own
  document would, and can never make one user's ranking depend on another's
  tools.
- **Durable store** (`FilesystemToolVectorStore`, bound through a
  `ToolVectorStoreSlot`). A typed wrapper over `ScopedFilesystem` under the
  per-user `/tool-vectors` alias, which composition resolves to
  `/tenants/<tenant>/users/<user>/tool-vectors`. One file per vector,
  `/tool-vectors/<space>/<sha256>.f32` (little-endian `f32`s), where
  `<space>` hashes the embedding provider id, model and configured dimension:
  a model or dimension change reads nothing the old configuration wrote. A
  per-owner manifest (`/tool-vectors/manifest.json`, updated only through
  `cas_update`) records each vector's last-used day and bounds the owner to
  4096 vectors across spaces, evicting the least recently used first. Saves
  write the manifest before the file and evictions delete the file before
  the manifest entry, so a crash leaves at most a manifest entry without a
  file, which reads as a miss. Only owner-scoped fits
  (`fit_for_owner`) read or write it; an owner-blind `fit` stays in memory.
- **Background jobs.** Missing documents are embedded by jobs the provider
  owns, in batches of 32; each batch is cached and persisted as it lands.
  A fit only waits for its jobs, so a caller that stops waiting (the hybrid
  ranker's 5 s fit timeout) drops the wait, never the work: the next fit finds
  the finished batches. A document is in at most one job at a time, at most
  two jobs embed at once and at most sixteen exist; past that a fit fails as
  `unavailable` (hybrid ranking falls back to BM25F for that fit) and
  background requests are dropped until the queue drains. Dropping the
  provider aborts its jobs.
- **Ahead of turns.** `index_in_background(owner, definitions)` runs the same
  lookup and embedding without a waiting fit. The loop host's
  `ToolCatalogIndexer` calls it for each known user once at startup and again
  whenever the active extension registry changes (install, activate, update,
  remove, restore, MCP re-discovery), so a new conversation's fit usually finds
  every vector stored.

## Scope: why per user

The authorized corpus is computed per user (extension grants are
owner-filtered, and MCP servers can be private to one user), so the vectors
derived from it are stored with the same scope. A fit for one user reads only
that user's store: a tool document from one user's private server is never
read back for another user's corpus, and no user can probe, by timing, what
another has stored. The cost is that tools every user shares are embedded
once per user rather than once per deployment. Keeping tenant and user in the
path also follows the filesystem contract (tenant/user keys stay in the path,
so per-tenant mounts route them).

## Compatibility and rollback

The store is a derived cache, never LLM data: deleting `/tool-vectors` (or any
part of it) only costs a re-embed. An older build that predates it ignores the
directory. A model, provider or dimension change starts a new space; the old
space's vectors are never read and are evicted by the size bound as the new
ones need room. The manifest carries a schema version; a build that finds a
version it does not know treats the manifest as unreadable, stores nothing new
for that user, and embeds per fit as before.

## Scores (`dense-cosine-v1`)

Cosine similarity with negative values clamped to zero, so scores lie in
`[0, 1]`. Tools scoring zero are not returned. A query equal to a tool's
capability id or provider tool name ranks that tool first at `1.0`. Ties break
on capability id, so ranking is deterministic across fits and input order.
Because the scale starts at zero, a relative threshold (score divided by the
top score of the same search) keeps its meaning.

## Confidentiality

Every tool document and every search query is sent to the configured
embedding endpoint. **Use a local endpoint** (Ollama, llama.cpp, vLLM on the
same host or network) unless sending tool schemas and user search text to a
third party is acceptable. Queries and schema text are never logged and never
appear in an error: failures carry fixed text plus, at most, an HTTP status
or a vector dimension.

## Tests

`cargo test -p ironclaw_tool_retrieval` — a bag-of-words stub embedder drives
ranking, cache reuse, catalog changes, determinism, error mapping, and a log
capture proving neither query nor schema text is logged.
`tests/vector_persistence.rs` runs the store contract on the in-memory and
libSQL backends (with a reopen) and on PostgreSQL (at
`IRONCLAW_TOOL_VECTOR_STORE_POSTGRES_URL`, else a throwaway container; skipped
without Docker), and drives the provider
through a restart, a model change, a timed-out fit, owner isolation,
background indexing and a burst of indexing requests.
