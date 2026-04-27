# Reborn Contract Freeze Index

**Status:** Contract-freeze packet draft  
**Date:** 2026-04-25  
**Purpose:** freeze system-wide ownership and interface decisions so implementation can be delegated in parallel.

---

## 1. What contract freeze means

Contract freeze does **not** mean every implementation is complete.

Contract freeze means an engineer can pick up a task and know:

1. which crate/service owns the domain;
2. which crates must not depend on it;
3. which public traits/types are stable enough to implement against;
4. where durable state lives;
5. which scope fields must flow through the call;
6. which side effects happen, and in what order;
7. which failures are fail-closed vs best-effort;
8. which errors/events must be redacted;
9. which tests prove the contract.

If a task needs to change one of those answers, it is not implementation work; it is a contract-change request.

---

## 2. Frozen cross-system decisions

| Area | Decision |
| --- | --- |
| Global scope | Preserve optional `AgentId` as a first-class scope alongside tenant/user/project/mission/thread/process/invocation. |
| Storage model | Hybrid: file-shaped content uses filesystem surfaces; structured/query-heavy/security/control-plane state uses typed repositories. |
| Namespace map | Adopt the map in [`storage-placement.md`](storage-placement.md). |
| Filesystem V1 API | `read_file`, `write_file`, `list_dir`, `stat`, `delete`, `create_dir_all`. CAS/streaming/rename/append are deferred. |
| Memory service shape | Split services over shared memory backend: document/search/prompt/seed/profile/layer/version services. |
| Memory multi-scope | Production-like explicit read scopes; writes primary by default; identity/system-prompt files primary-only. |
| Memory layers | Include in V1, but layer scopes must be namespaced and non-colliding with raw user IDs. |
| Prompt context | `MemoryPromptContextService` owns prompt assembly and prompt-injection write safety. |
| Secrets | Typed encrypted secret repository is production source of truth; file views are redacted projections/reference only. |
| Network | All host/provider HTTP goes through `ironclaw_network`. |
| Events/projections | V1 includes durable append log, projections, SSE/WebSocket, and replay cursors. |
| Resources | V1 reserves/enforces runtime/process, network, embeddings/providers, and artifacts/storage quotas. |
| Settings/extensions/skills | Typed repositories are source of truth with optional `/system/...` file projections. |
| Extensions | Full lifecycle contract is frozen; partial implementation is allowed if states/transitions remain compatible. |
| Agent loop | First-party `TurnService`/`AgentLoopService` using `CapabilityHost`. |
| Processes | Status/kill/await/result/output-ref plus streaming events in V1. |
| Approvals | Exact-invocation leases only in V1; reusable scoped approvals are V2. |
| Obligations | All built-in obligations must be implemented for V1; unsupported obligations fail closed. |
| Migration | Reuse existing schemas where viable; bridge only when necessary. |
| Runtime lanes | WASM, Script, and MCP are all first-class V1 lanes. |

---

## 3. Contract document packet

### Existing contracts to treat as active

- [`host-api.md`](host-api.md)
- [`capability-access.md`](capability-access.md)
- [`capabilities.md`](capabilities.md)
- [`approvals.md`](approvals.md)
- [`run-state.md`](run-state.md)
- [`dispatcher.md`](dispatcher.md)
- [`runtime-workflows.md`](runtime-workflows.md)
- [`wasm.md`](wasm.md)
- [`scripts.md`](scripts.md)
- [`mcp.md`](mcp.md)
- [`processes.md`](processes.md)
- [`filesystem.md`](filesystem.md)
- [`secrets.md`](secrets.md)
- [`network.md`](network.md)
- [`events.md`](events.md)
- [`events-projections.md`](events-projections.md)
- [`resources.md`](resources.md)
- [`extensions.md`](extensions.md)

### New contracts in this packet

- [`storage-placement.md`](storage-placement.md)
- [`memory.md`](memory.md)
- [`settings-config.md`](settings-config.md)
- [`turns-agent-loop.md`](turns-agent-loop.md)
- [`migration-compatibility.md`](migration-compatibility.md)

---

## 4. Delegation readiness checklist

A task is ready to hand to an engineer only if its prompt includes:

```text
Contract doc path(s)
Target crate(s)
Source-of-truth storage location
Scope fields to propagate
Forbidden dependencies
Fail-closed cases
Best-effort cases
Redaction/no-leak requirements
PostgreSQL/libSQL parity requirement, if applicable
Migration/doc update requirement
Acceptance tests
Verification commands
```

Every task touching authority, persistence, events, network, secrets, filesystem, memory, approvals, or runtime execution must include at least one caller-level test. A helper-only test is insufficient when a helper gates a side effect.

---

## 5. Parallel implementation waves

### Wave 0 — contract ratification

Goal: make docs explicit enough that implementation tasks do not need architectural debate.

Tasks:

1. Add `AgentId` to `ironclaw_host_api` scope/resource/event shapes.
2. Finalize `RootFilesystem::delete` and `RootFilesystem::create_dir_all` semantics.
3. Ratify memory service trait shapes from [`memory.md`](memory.md).
4. Ratify durable event cursor envelope from [`events-projections.md`](events-projections.md).
5. Ratify settings/config source-of-truth rules from [`settings-config.md`](settings-config.md).

### Wave 1 — independent substrate tasks

Can run in parallel after Wave 0 docs are accepted:

| Task | Main contract | Primary crate(s) |
| --- | --- | --- |
| Filesystem V1 ops | `filesystem.md` | `ironclaw_filesystem` |
| AgentId propagation | `host-api.md`, `storage-placement.md` | `ironclaw_host_api`, all scope stores |
| Typed secret repository | `secrets.md` | `ironclaw_secrets` |
| Network provider client boundary | `network.md` | `ironclaw_network`, provider crates |
| Durable event log/cursors | `events-projections.md` | `ironclaw_events`, web gateway later |
| Resource reservation expansion | `resources.md` | `ironclaw_resources`, capabilities/processes/network |
| Extension lifecycle state machine | `extensions.md` | `ironclaw_extensions` |

### Wave 2 — memory/workspace parity

Can run in parallel once `memory.md` is accepted:

| Task | Main contract | Notes |
| --- | --- | --- |
| `MemoryDocumentService` | `memory.md` | read/write/append/delete/list/exists over backend |
| `MemorySearchService` | `memory.md` | multi-scope search, identity filtering, search config |
| `MemoryPromptContextService` | `memory.md`, `turns-agent-loop.md` | prompt assembly + sanitizer policy |
| `MemorySeedService` | `memory.md` | core seeds, bootstrap, `.config` seeds, imports |
| `MemoryLayerService` | `memory.md` | namespaced layers + privacy redirect |
| `MemoryVersionService` | `memory.md` | get/list/prune/patch version behavior |
| Embedding provider adapters | `memory.md`, `network.md`, `secrets.md` | OpenAI/Ollama/NEAR/Bedrock via policy-aware clients |

### Wave 3 — product integration

| Task | Main contract | Notes |
| --- | --- | --- |
| `TurnService`/`AgentLoopService` | `turns-agent-loop.md` | one-active-run-per-thread, prompt context, CapabilityHost |
| Web SSE/WebSocket event APIs | `events-projections.md` | durable replay cursors + projections |
| Settings/extension/skill projections | `settings-config.md`, `extensions.md` | typed repos with `/system/...` views |
| Runtime lane hardening | `wasm.md`, `scripts.md`, `mcp.md`, `network.md` | all three first-class |
| Migration bridge | `migration-compatibility.md` | reuse schemas where viable |

---

## 6. Non-negotiable implementation invariants

- `CapabilityHost` is the caller-facing workflow gate; callers do not manually authorize then call dispatcher.
- `RuntimeDispatcher` routes already-authorized runtime requests only.
- Unsupported obligations fail closed before runtime dispatch, process start, approval lease claim, secret consumption, or network execution.
- Event sink delivery failures are best-effort; audit/persistence failures are domain-specific and must be explicit.
- Raw secrets, host paths, unapproved input/output, approval reasons, lease contents, and backend error details must not appear in user-facing errors/events/audit.
- Tenant/user/project/agent scope must flow through persistence, resources, events, approvals, leases, processes, results, outputs, secrets, network, runtime boundaries, and memory routing.
- PostgreSQL/libSQL parity is required for production persistence behavior unless a contract explicitly says a backend is unsupported.
- `ironclaw_filesystem` remains generic and must not learn memory-domain path grammar.
- `ironclaw_memory` owns memory path grammar, memory backend plugin contracts, metadata/search/indexing, and prompt-context policy services.
- Provider HTTP and embedding/memory adapter network calls must go through `ironclaw_network`.

---

## 7. Review rubric for delegated work

A delegated implementation is not complete until it provides:

1. narrow unit tests for pure transforms;
2. caller/service-level tests for side-effect paths;
3. tenant/user/project/agent isolation tests where scoped persistence is touched;
4. redaction/no-leak tests for errors/events/audit if sensitive data is touched;
5. PostgreSQL and libSQL tests for shared production repositories;
6. dependency-boundary checks for Reborn crate rules;
7. docs updates for any contract behavior changed;
8. targeted `cargo fmt`, `cargo test`, `cargo clippy`, `cargo doc` evidence for touched crates.
