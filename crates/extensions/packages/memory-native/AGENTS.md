# Agent Map — ironclaw_memory_native

Canonical working-rules file for this package crate (`CLAUDE.md` here is a
pointer; consolidated 2026-08-05 per `docs/internal/reborn/guidance-conventions.md`
rule 1).

## Start Here

- Read `README.md` for orientation, `Cargo.toml` for actual dependencies.
- This directory is the whole **memory-native package**: crate, `manifest.toml`
  (declaring the `[memory]` provider surface), `prompts/`, and `schemas/`
  together, per the family's self-containment rule.
- The provider-neutral contract (the `MemoryService` trait, DTOs,
  scope/path/context value types, prompt-safety vocabulary, audit/event
  contracts) lives in `crates/domains/ironclaw_memory`, which this crate
  implements and re-exports. Contracts of record:
  - `docs/internal/reborn/contracts/memory.md`
  - `docs/internal/reborn/contracts/storage-placement.md`
  - `docs/internal/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- The memory-document system over host-resolved scope:
  - Document repositories + backend plugin contracts:
    `MemoryDocumentRepository` with `FilesystemMemoryDocumentRepository` /
    `InMemoryMemoryDocumentRepository`, `MemoryBackend` /
    `RepositoryMemoryBackend` / `MemoryBackendCapabilities` (`repo`, `backend`).
  - `/memory` virtual path grammar and scope: `MemoryDocumentPath`,
    `MemoryDocumentScope` (`path`); document metadata/options this crate
    consumes from `ironclaw_memory` (`DocumentMetadata`, `HygieneMetadata`,
    `CONFIG_FILE_NAME`) plus this crate's own `MemoryWriteOptions`
    (`metadata`) and internal schema validation (`schema`).
  - Chunking + content hashing (`ChunkConfig`, `chunk_document`,
    `content_sha256`) and the indexer hooks (`chunking`, `indexer`) — no
    embedding-provider port exists; vector search only activates against
    pre-supplied embeddings (see `backend.rs`'s fail-closed messages).
  - Hybrid search (FTS + vector via RRF fusion): `MemorySearchRequest`,
    `MemorySearchResult`, `FusionStrategy` (`search`).
  - The memory-document filesystem adapter (`filesystem`), the
    significant-event sink (`events`), and the prompt-write safety engine
    (`PromptWriteSafetyPolicy` + protected-path/decision/event types, `safety`)
    that implements the vocabulary the neutral contract defines.
- The model-facing memory **guidance** this provider ships
  (`prompts/memory-guidance.md`, declared as `[memory].guidance_doc`): when a
  durable fact is worth saving, how to phrase it, what never to save. It is
  provider-owned because it names this provider's tools and describes this
  provider's recall behavior; the host appends whatever the bound provider
  declares and writes none of it. A provider that ships none has nothing
  appended (mem0 declares none on purpose).
- The always-on curated prefix of `read_long_term`: this provider serves its
  standing `MEMORY.md` at the head of its own long-term lane, ahead of the
  full-text hits and independent of the turn's query (#7185). Budget, line
  splitting, and the truncation marker are this provider's policy — the host
  sees ordinary lane snippets and knows no document paths.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Guardrails

- Depend on exactly `ironclaw_memory`, `ironclaw_filesystem`,
  `ironclaw_safety`, `ironclaw_host_api` — the measured set today (the family
  spec additionally sanctions `ironclaw_extension_contracts`; add it only when
  a surface genuinely needs it). Do not depend on the mem0 package, any HTTP
  client, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any
  kernel or product crate.
- Memory backends are plugins behind host-resolved scope. They must not infer
  broader tenant/user/agent/project authority or bypass mount/scoped-filesystem
  checks.
- Keep semantic search, chunking, embeddings, and versioning behind
  memory-owned repository/indexer abstractions; generic mount/catalog logic
  stays in `ironclaw_filesystem`.
- Every read/list/search/write/version/chunk operation filters by the full
  `(tenant_id, user_id, agent_id, project_id)` tuple. Do not infer project
  scope from path prefixes. Document uniqueness is
  `UNIQUE (tenant_id, user_id, agent_id, project_id, path)`.
- Use the empty string as the storage-only absent sentinel for `agent_id` and
  `project_id` (safe because `MemoryDocumentScope` rejects empty supplied ids).
  `_none` is the **virtual-path** sentinel only; never store it.
- Capability declarations (`MemoryBackendCapabilities`) are enforcement inputs:
  unsupported file/search behavior fails closed before backend side effects.
- Treat document writes as committed once persistence succeeds: a derived
  index/embedding refresh failure after persistence must not make the write
  report failure.
- Persistence is a single `FilesystemMemoryDocumentRepository` layered on
  `RootFilesystem`; the in-memory repository is test support, never a
  deployment target. Backend-specific (libSQL/Postgres) behavioral coverage
  belongs in `ironclaw_filesystem`'s backend contract tests; this crate's tests
  target the in-memory backend and exercise memory-document semantics
  (versioning, chunk replace, metadata cascade, hybrid search fusion).

## Do Not Move In Here

- The neutral `MemoryService` vocabulary (stays in `ironclaw_memory`).
- Generic filesystem semantics, direct provider HTTP, raw secret handling, or
  loop prompt strategy.
- Secrets, raw host paths, backend error details, or unredacted user content
  in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_memory_native` (contract suites in
  `tests/`: memory_service / memory_backend / memory_filesystem / repo_*).
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`.
- The shared `MemoryService` conformance suite also runs against the mem0
  package — keep both providers passing it when the contract shifts.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a
  neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network,
  secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a
  contract-change request instead of silently changing ownership.
