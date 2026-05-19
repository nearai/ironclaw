# Reborn Contract — Memory Product Surfaces

**Status:** First-slice contract and inventory
**Issue:** #3287
**Date:** 2026-05-19

---

## Purpose

#3287 migrates memory-document product behavior onto focused Reborn memory
services without recreating the v1 monolithic `Workspace` as the Reborn source
of truth.

This document covers product-surface mapping only. Physical storage, schema,
backfill, readiness, and production cutover remain owned by #3118, #3029, and
their follow-ups.

The first implementation slice is:

- focused service contracts in `ironclaw_memory`;
- an adapter-safe `MemoryProductFacade` in `ironclaw_product_workflow`;
- fake-service contract tests proving path, scope, policy, and service routing;
- guardrails preventing v1 `Workspace` or raw substrate dependencies from
  entering the Reborn product memory facade.

## Product Boundary

Product callers use legacy-relative paths such as:

```text
MEMORY.md
daily/2026-05-19.md
projects/alpha/notes.md
context/profile.json
```

Product callers must not construct or expose scoped internal paths such as:

```text
/memory/tenants/{tenant}/users/{user}/agents/{agent}/projects/{project}/...
```

`MemoryProductFacade` receives a resolved `MemoryDocumentScope` from product or
binding context and passes `(scope, relative_path)` into typed memory services.
It rejects local-filesystem-looking paths before service calls.

## Service Ownership

| Owner service | Product behavior |
| --- | --- |
| `MemoryDocumentService` | current read, write, append, patch, list, tree, status |
| `MemorySearchService` | search options, full-text/vector/hybrid handling, embedding/query generation |
| `MemoryLayerService` | private/shared/custom layer writes, force flag, privacy redirects |
| `MemoryVersionService` | version list and specific version read |
| `MemorySeedService` | bootstrap clear and future seed/upgrade flows |
| `MemoryProfileService` | profile-derived document sync |
| `MemoryPromptWriteSafetyPolicy` | product-level protected prompt-path gate with actor, surface, and purpose |

`MemoryPromptContextService` / #3091 owns loop prompt assembly. #3287 supplies
scoped memory service data and product surface behavior only.

## Behavior Inventory

| Surface | Current v1 path | Reborn owner | Scope inputs | Path handling | Compatibility |
| --- | --- | --- | --- | --- | --- |
| CLI `memory search` | `src/cli/memory.rs` → `Workspace::search_with_config` | `MemorySearchService` | tenant/user/agent/project from CLI product context | query + limit only; no embedding in product layer | preserve text output shape where practical; sanitized unavailable errors |
| CLI `memory read` | `src/cli/memory.rs` → `Workspace::read` | `MemoryDocumentService` / `MemoryVersionService` | primary scope | relative path | preserve content output and not-found semantics |
| CLI `memory write --append` | `src/cli/memory.rs` → `Workspace::append` | `MemoryDocumentService` | primary scope | relative path | preserve "Appended to {path}" success; protected paths go through policy |
| CLI `memory write` replace | `src/cli/memory.rs` → `Workspace::write` | `MemoryDocumentService` | primary scope | relative path | preserve "Wrote to {path}" success; protected paths go through policy |
| CLI `memory tree` | `src/cli/memory.rs` → `Workspace::list` recursion | `MemoryDocumentService` | configured read scope policy | relative root | preserve directory-like display |
| CLI `memory status` | `src/cli/memory.rs` → `Workspace::list_all` / `exists` | `MemoryDocumentService` | primary scope | key identity paths | preserve counts/key-file checks where practical |
| Web `GET /api/memory/tree` | `src/channels/web/handlers/memory.rs` → `Workspace::list_all` | `MemoryDocumentService` | authenticated user scope | relative paths only | preserve `TreeEntry { path, is_dir }` |
| Web `GET /api/memory/list` | `src/channels/web/handlers/memory.rs` → `Workspace::list` | `MemoryDocumentService` | authenticated user scope | relative parent | preserve `ListEntry` fields |
| Web `GET /api/memory/read` | `src/channels/web/handlers/memory.rs` → `Workspace::read` | `MemoryDocumentService` / `MemoryVersionService` | authenticated user scope | relative path | preserve `path/content/updated_at` success shape where practical |
| Web `POST /api/memory/write` | `src/channels/web/handlers/memory.rs` → `Workspace::write/append` | `MemoryDocumentService` / `MemoryLayerService` | authenticated user scope | relative path + optional layer | preserve `status`, `redirected`, `actual_layer`; policy differs by actor/surface |
| Web `POST /api/memory/search` | `src/channels/web/handlers/memory.rs` → `Workspace::search` | `MemorySearchService` | authenticated user scope | query + limit only | preserve hit fields where practical; sanitize provider/backend failures |
| Tool `memory_search` | `src/tools/builtin/memory.rs` → `WorkspaceResolver` + `Workspace::search` | HostRuntime capability execution → `MemorySearchService` | host-mediated capability scope | query + limit; optional reasoning stays outside service contract until #3090/#3016 | tool output keeps `query/results/result_count`; no raw provider errors |
| Tool `memory_read` current | `src/tools/builtin/memory.rs` → `Workspace::read` | HostRuntime capability execution → `MemoryDocumentService` | host-mediated capability scope | relative path; reject filesystem-looking paths | preserve `path/content/word_count/updated_at` |
| Tool `memory_read` versions | `src/tools/builtin/memory.rs` → `Workspace::list_versions/get_version` | HostRuntime capability execution → `MemoryVersionService` | host-mediated capability scope | relative path + version mode | preserve version list/read fields |
| Tool `memory_tree` | `src/tools/builtin/memory.rs` → `Workspace::list` recursion | HostRuntime capability execution → `MemoryDocumentService` | host-mediated capability scope | relative root + depth | preserve compact tree output |
| Tool `memory_write target=memory` | `src/tools/builtin/memory.rs` → `append_memory` or `write(MEMORY.md)` | HostRuntime capability execution → `MemoryDocumentService` + prompt policy | host-mediated capability scope | shortcut resolves to `MEMORY.md` | protected prompt-path policy required |
| Tool `memory_write target=daily_log` | `src/tools/builtin/memory.rs` → `append_daily_log_tz` | HostRuntime capability execution → `MemoryDocumentService` | host-mediated capability scope + local date | shortcut resolves to `daily/YYYY-MM-DD.md` | preserve append-oriented default |
| Tool `memory_write target=heartbeat` | `src/tools/builtin/memory.rs` → `HEARTBEAT.md` | HostRuntime capability execution → `MemoryDocumentService` + prompt policy | host-mediated capability scope | shortcut resolves to `HEARTBEAT.md` | protected prompt-path policy required |
| Tool `memory_write target=bootstrap` | `src/tools/builtin/memory.rs` → clear `BOOTSTRAP.md` and mark bootstrap complete | HostRuntime capability execution → `MemorySeedService` + prompt policy | host-mediated capability scope | shortcut resolves to `BOOTSTRAP.md` | content ignored; success reports cleared |
| Tool `memory_write target=<path>` | `src/tools/builtin/memory.rs` → `Workspace::write/append` | HostRuntime capability execution → document/layer/profile services | host-mediated capability scope | custom relative path only | reject local filesystem-looking paths |
| Tool patch mode | `src/tools/builtin/memory.rs` → `Workspace::patch` | HostRuntime capability execution → `MemoryDocumentService` | host-mediated capability scope | relative path + exact old/new strings | old string cannot be empty; layer + patch rejected |
| Metadata writes | `src/tools/builtin/memory.rs` applies metadata before write | `MemoryDocumentService` / `MemoryLayerService` | actor/surface/purpose authority | relative path | preserve `skip_indexing`, `skip_versioning`, hygiene, schema metadata |
| Layer writes | v1 `Workspace::write_to_layer/append_to_layer` | `MemoryLayerService` | canonical layer ref; no raw user-id layer authority | relative path + layer + force | preserve redirect result and read-only errors |
| Profile writes | v1 `sync_profile_documents` after `context/profile.json` | `MemoryProfileService` | primary scope | `context/profile.json` | preserve USER/directive derived-doc sync |
| Seed/bootstrap | v1 seed files under `src/workspace/seeds` and bootstrap clear behavior | `MemorySeedService` | setup/admin or tool authority | protected relative paths | no production migration in first slice |
| Prompt context | v1 workspace prompt assembly | #3091 `MemoryPromptContextService` | primary scope + explicit secondary scopes/group policy | identity docs primary-only | #3287 does not implement ordering/formatting |

## Privacy And Policy Rules

- Identity and prompt-context files are primary-scope only.
- Secondary memory scopes may contribute ordinary memory reads/search results,
  but not identity/profile/system prompt documents by default.
- Group-chat prompt context excludes personal memory/profile/directives unless
  explicit policy allows it.
- Protected prompt-path writes carry:
  - actor (`user`, `agent`, `admin`, or `tool`);
  - surface (`cli`, `web`, `llm_tool`, `setup_admin`, or `prompt_context`);
  - purpose (`memory`, `daily_log`, `heartbeat`, `bootstrap`, `custom_path`,
    `metadata`, `layer_write`, `profile_sync`, or `seed`).
- Policy and service errors must be sanitized; raw host paths, provider details,
  SQL strings, tokens, or secret-like values must not cross the product facade.

## Coexistence

Until migration/readiness work lands:

- legacy `Workspace` remains the production writer;
- Reborn product memory surfaces remain contracts/fakes/default-off;
- comparison bridges, if any, are read-only or dry-run validation;
- no silent legacy/Reborn dual-write or hidden fallback writer is allowed.

## First-Slice Test Coverage

The first implementation tests cover:

- shortcut resolution for `memory` and `bootstrap`;
- rejection of filesystem-looking paths before service calls;
- protected prompt-path policy calls with actor/surface/purpose;
- layer write routing with force and metadata;
- version-list routing;
- identity prompt doc primary-scope enforcement;
- search request options and secondary identity exclusion;
- profile-derived document sync;
- sanitized service errors.

Future production route/tool PRs must add caller-level tests at the actual
handler/tool entrypoint, not only helper tests.
