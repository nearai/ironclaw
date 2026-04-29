# ironclaw_memory guardrails

- Own memory/workspace document repository seams, `/memory` virtual path grammar, memory-document filesystem adapters, and indexer hook boundaries.
- Depend on `ironclaw_host_api` and `ironclaw_filesystem`; do not move generic mount/catalog logic here.
- Do not depend on product workflow, dispatcher, concrete runtimes, approvals, run-state, secrets, network, process, events, or extension crates.
- Keep semantic search, chunking, embeddings, and versioning behind memory-owned repository/indexer abstractions; do not put them in `ironclaw_filesystem`.
- PostgreSQL/libSQL repository adapters may map file-shaped memory documents into the existing `memory_documents` table shape, but chunk/search/embedding updates must remain explicit indexer/service work.
- Preserve tenant/user/project scope on every path parse and repository operation.
- Treat `_none` as the virtual path sentinel for absent project ids; never store it as a real project id.
