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
| Kernel boundary | Kernel means the security perimeter that mediates authority; loops, prompt assembly, mission/routine behavior, skill selection, and channel behavior are userland. See [`kernel-boundary.md`](kernel-boundary.md). |
| Global scope | Preserve optional `AgentId` as a first-class scope alongside tenant/user/project/mission/thread/process/invocation. |
| Storage model | Hybrid: file-shaped content uses filesystem surfaces; structured/query-heavy/security/control-plane state uses typed repositories. |
| Namespace map | Adopt the map in [`storage-placement.md`](storage-placement.md). |
| Filesystem V1 API | `read_file`, `write_file`, `append_file`, `list_dir`, `stat`, `delete`, `create_dir_all`. CAS/streaming/rename are deferred. |
| Memory service shape | Split services over shared memory backend: document/search/prompt/seed/profile/layer/version services. |
| Memory multi-scope | Production-like explicit read scopes; writes primary by default; identity/system-prompt files primary-only. |
| Memory layers | Include in V1, but layer scopes must be namespaced and non-colliding with raw user IDs. |
| Prompt context | Prompt-injection write safety is kernel-mediated policy; prompt assembly is loop/userland strategy over authorized memory reads. |
| Secrets | Typed encrypted secret repository is production source of truth; file views are redacted projections/reference only. |
| Network | All host/provider HTTP goes through `ironclaw_network`. |
| Events/projections | Durable append log plus scoped replay cursors are substrate dependencies; projections and SSE/WebSocket transports build on that substrate. |
| Resources | V1 reserves/enforces runtime/process, network, embeddings/providers, and artifacts/storage quotas. |
| Settings/extensions/skills | Typed repositories are source of truth with optional `/system/...` file projections. |
| Extensions | Full lifecycle contract is frozen; partial implementation is allowed if states/transitions remain compatible. |
| Agent loop | Shipped loop docs describe reference userland loops running on the kernel surface; no loop gets a privileged bypass. |
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

- [`kernel-boundary.md`](kernel-boundary.md)
- [`storage-placement.md`](storage-placement.md)
- [`memory.md`](memory.md)
- [`settings-config.md`](settings-config.md)
- [`turns-agent-loop.md`](turns-agent-loop.md)
- [`migration-compatibility.md`](migration-compatibility.md)

---

## 4. Current implementation status

The implementation-alignment map is:

```text
docs/reborn/2026-04-25-current-architecture-map.md
```

Reviewers should use it alongside this packet to separate:

```text
[contract exists]         contract/docs/API shape exists; runnable implementation may still be pending
[implemented slice]      tested implementation exists for the described slice; not a blanket product-complete claim
[partially implemented]  subset exists, but product/production work remains
[fully implemented]      complete for the frozen V1 contract; use only when the whole contract is done
[not implemented]        intentionally missing or deferred
```

Unless a row explicitly says `[fully implemented]`, reviewers should read the status as applying only to the narrow slice described, not as a full product-completion claim.

Current implemented/partial substrate called out there includes:

- host API vocabulary and neutral dispatch contracts;
- root/scoped/composite filesystem surfaces and DB-backed root filesystem backends;
- `ironclaw_memory` backend/filesystem adapter, DB repositories, metadata/search, embeddings, and plugin seam;
- extension discovery/registry contracts;
- resource governor primitives;
- secret metadata/encryption/leases plus filesystem-backed repository reference;
- network policy boundary and hardened WASM host HTTP path;
- capability access, `CapabilityHost`, approvals/resume, and run-state slices;
- dispatcher runtime-adapter inversion;
- WASM, Script, and MCP adapter lanes;
- process store/manager/result/output-ref/process-host slices;
- architecture dependency guardrails and live vertical-slice examples.

Explicit gaps are also listed there, including kernel trust-class policy engine productization, turn coordination/reference loop services, durable event projections/SSE/WebSocket, full obligation implementations, production typed secret repository wiring, product memory service parity, and migration bridges.

---

## 5. Delegation readiness checklist

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

## 6. Implementation dependency graph, not calendar waves

The levels below are dependency levels, not a delivery schedule. With agent-assisted implementation, downstream work should fan out as soon as the contracts it depends on are ratified. Do not wait for every item in a numbered bucket if a task's direct dependencies are already frozen.

Dependency rules:

1. Contract ratification gates implementation only for the contracts a task depends on.
2. Substrate tasks with independent contracts can run concurrently.
3. Product integration tasks should wait for their substrate dependencies, not for unrelated substrate tasks.
4. If a task requires changing frozen ownership, scope, storage placement, or failure semantics, it is a contract-change request rather than implementation work.

### Level 0 — contract ratification

Goal: make docs explicit enough that implementation tasks do not need architectural debate.

Tasks:

1. Ratify the kernel/userland boundary and trust-class policy from [`kernel-boundary.md`](kernel-boundary.md).
2. Add `AgentId` to `ironclaw_host_api` scope/resource/event shapes.
3. Finalize `RootFilesystem::append_file`, `RootFilesystem::delete`, and `RootFilesystem::create_dir_all` semantics.
4. Ratify memory service trait shapes from [`memory.md`](memory.md), including the split between prompt safety policy and prompt assembly strategy.
5. Ratify durable event cursor envelope from [`events-projections.md`](events-projections.md).
6. Ratify settings/config source-of-truth rules from [`settings-config.md`](settings-config.md).

### Level 1 — independent substrate tasks

Can run in parallel after their direct Level 0 contract dependencies are accepted:

| Task | Main contract | Primary crate(s) | Direct blockers |
| --- | --- | --- | --- |
| Filesystem V1 ops | `filesystem.md` | `ironclaw_filesystem` | filesystem ops semantics |
| AgentId propagation | `host-api.md`, `storage-placement.md` | `ironclaw_host_api`, all scope stores | global scope model |
| Typed secret repository | `secrets.md` | `ironclaw_secrets` | storage/source-of-truth rules |
| Network provider client boundary | `network.md` | `ironclaw_network`, provider crates | network boundary contract |
| Durable event log/cursors | `events-projections.md` | `ironclaw_events`, web gateway later | cursor envelope and redaction rules |
| Resource reservation expansion | `resources.md` | `ironclaw_resources`, capabilities/processes/network | resource scope and lifecycle ownership |
| Extension lifecycle state machine | `extensions.md` | `ironclaw_extensions` | lifecycle states/transitions |
| Trust-class policy engine | `kernel-boundary.md`, `host-api.md`, `extensions.md` | host policy/composition + extension registry | trust assignment, upgrade, revocation, grant ceilings |

`Durable event log/cursors` is a substrate dependency, not merely a web feature. It gives parallel implementation agents a typed, replayable surface for caller-level tests and cross-component debugging. SSE/WebSocket transport can remain downstream product integration over this substrate.

### Level 2 — memory/workspace parity

Can run in parallel once the relevant `memory.md`, storage, network, secrets, and event substrate dependencies are accepted:

| Task | Main contract | Notes |
| --- | --- | --- |
| `MemoryDocumentService` | `memory.md` | read/write/append/delete/list/exists over backend |
| `MemorySearchService` | `memory.md` | multi-scope search, identity filtering, search config |
| Prompt safety policy + reference prompt assembler | `memory.md`, `kernel-boundary.md`, `turns-agent-loop.md` | kernel-mediated write safety, plus replaceable loop-owned prompt assembly over authorized reads |
| `MemorySeedService` | `memory.md` | core seeds, bootstrap, `.config` seeds, imports |
| `MemoryLayerService` | `memory.md` | namespaced layers + privacy redirect |
| `MemoryVersionService` | `memory.md` | get/list/prune/patch version behavior |
| Embedding provider adapters | `memory.md`, `network.md`, `secrets.md` | OpenAI/Ollama/NEAR/Bedrock via policy-aware clients |

### Level 3 — coherent product integration

| Task | Main contract | Notes |
| --- | --- | --- |
| Kernel turn coordination + reference loops | `kernel-boundary.md`, `turns-agent-loop.md` | one-active-run-per-thread in kernel-mediated coordination; loop behavior/prompt strategy as userland over `CapabilityHost` |
| Web SSE/WebSocket event APIs | `events-projections.md` | product transport over durable replay cursors + projections |
| Settings/extension/skill projections | `settings-config.md`, `extensions.md` | typed repos with `/system/...` views |
| Runtime lane hardening | `wasm.md`, `scripts.md`, `mcp.md`, `network.md` | all three first-class |
| Migration bridge | `migration-compatibility.md` | reuse schemas where viable |

---

## 7. Cutover discipline

Reborn implementation slices can land incrementally, but user-visible exposure should cut over coherently behind a feature flag or parallel binary rather than as disconnected islands. The target is a single `vReborn` composition path where `HostRuntimeServices` is built from config and caller-level tests exercise the integrated `CapabilityHost -> Dispatcher -> Adapter -> Process/Event/Memory` chains.

Legacy `src/` feature additions are a drift risk while Reborn is being built. This packet does not enact a blanket `src/` feature freeze; security fixes, urgent customer fixes, and explicitly approved compatibility work may continue. New non-trivial product work should prefer Reborn crates when practical, and a separate guardrail/CI task may be ratified later to flag additions to deprecated legacy modules.

---

## 8. Non-negotiable implementation invariants

- The kernel is the security perimeter, not the agent brain; anything not needed to mediate authority/security/coordination stays out of kernel-owned code.
- `CapabilityHost` is the caller-facing workflow gate; callers do not manually authorize then call dispatcher.
- `RuntimeDispatcher` routes already-authorized runtime requests only.
- Unsupported obligations fail closed before runtime dispatch, process start, approval lease claim, secret consumption, or network execution.
- Event sink delivery failures are best-effort; audit/persistence failures are domain-specific and must be explicit.
- Raw secrets, host paths, unapproved input/output, approval reasons, lease contents, and backend error details must not appear in user-facing errors/events/audit.
- Tenant/user/project/agent scope must flow through persistence, resources, events, approvals, leases, processes, results, outputs, secrets, network, runtime boundaries, and memory routing.
- PostgreSQL/libSQL parity is required for production persistence behavior unless a contract explicitly says a backend is unsupported.
- `ironclaw_filesystem` remains generic and must not learn memory-domain path grammar.
- Prompt-injection write safety is kernel-mediated policy; prompt assembly strategy belongs to the active loop over authorized memory reads.
- `ironclaw_memory` owns memory path grammar, memory backend plugin contracts, metadata/search/indexing, and prompt-context policy hooks.
- Provider HTTP and embedding/memory adapter network calls must go through `ironclaw_network`.
- Trust class is an authority ceiling and assignment policy input, not a permission grant or kernel bypass.
- Shipped and user-installed loops both run on the kernel surface and must use mediated grants, mounts, leases, resources, and obligations.

---

## 9. Review rubric for delegated work

A delegated implementation is not complete until it provides:

1. narrow unit tests for pure transforms;
2. caller/service-level tests for side-effect paths;
3. tenant/user/project/agent isolation tests where scoped persistence is touched;
4. redaction/no-leak tests for errors/events/audit if sensitive data is touched;
5. PostgreSQL and libSQL tests for shared production repositories;
6. dependency-boundary checks for Reborn crate rules;
7. docs updates for any contract behavior changed;
8. targeted `cargo fmt`, `cargo test`, `cargo clippy`, `cargo doc` evidence for touched crates.
