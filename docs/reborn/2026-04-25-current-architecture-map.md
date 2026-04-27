# IronClaw Reborn — Current architecture map

**Date:** 2026-04-25
**Generated:** 2026-04-25T12:18:38Z
**Status:** Current docs snapshot / implementation-alignment map
**Scope:** Reborn host architecture, current implemented slices, and explicit gaps

This document records the current Reborn shape after the recent architecture discussion. It is a map, not a replacement for the contract docs under `docs/reborn/contracts/`.

Terminology note: older docs use **kernel** for the small host-core composition concept. The current concrete crate for that concept is `ironclaw_host_runtime`; there is no active `ironclaw_kernel` crate in the Reborn stack.

Legend:

```text
[contract exists]         contract/docs/API shape exists; runnable implementation may still be pending
[implemented slice]      tested implementation exists for the described slice; not a blanket product-complete claim
[partially implemented]  subset exists, but product/production work remains
[fully implemented]      complete for the frozen V1 contract; use only when the whole contract is done
[not implemented]        intentionally missing or deferred
```

Unless a row explicitly says `[fully implemented]`, assume the status describes the narrow slice named in that row, not all product behavior for the area.

Contract freeze packet:

```text
docs/reborn/contracts/_contract-freeze-index.md
docs/reborn/contracts/storage-placement.md
docs/reborn/contracts/memory.md
docs/reborn/contracts/settings-config.md
docs/reborn/contracts/turns-agent-loop.md
docs/reborn/contracts/migration-compatibility.md
```

These docs record the delegation-ready system decisions: first-class optional `AgentId`, hybrid storage placement, typed repositories for structured state, split memory services over shared backends, durable event streams with replay cursors, all built-in obligations for V1, all three runtime lanes as first-class, and schema reuse where viable.

---

## 1. One host core, many ports/adapters

Reborn has one host core with many adapters and runtime ports. It should not grow one host per vendor or per transport.

```text
                               users / external systems

       +------------+   +------------+   +------------+   +------------+
       | CLI driver |   | Web driver |   | Slack drv  |   | Telegram   |
       | [adapter]  |   | [adapter]  |   | [adapter]  |   | [adapter]  |
       +-----+------+   +-----+------+   +-----+------+   +-----+------+
             |                |                |                |
             +----------------+----------------+----------------+
                              |
                              v
                  +---------------------------+
                  | TransportAdapter port     |
                  | normalize ingress/egress  |
                  | [contract; real channel   |
                  |  adapters mostly not yet] |
                  +-------------+-------------+
                                |
                                v
                  +---------------------------+
                  | Turn service              |
                  | one active run/thread,    |
                  | checkpoints, resume edge  |
                  | [not implemented]                 |
                  +-------------+-------------+
                                |
                                v
                  +---------------------------+
                  | First-party agent loop    |
                  | hosted service/extension  |
                  | emits Reply | Capability  |
                  | Calls [not implemented]           |
                  +-------------+-------------+
                                |
                                v
+-------------------------------+---------------------------------------+
|                         HOST CORE                                     |
|                                                                       |
|  +-------------------+       +-------------------+                    |
|  | CapabilityHost    | ----> | Authorization /   |                    |
|  | caller-facing     |       | grants / leases   |                    |
|  | workflow gate     |       | [partially implemented]  |                    |
|  | [implemented slice]          |       +---------+---------+                    |
|  +----+-------+------+                 |                              |
|       |       |                        v                              |
|       |       |              +-------------------+                    |
|       |       |              | Run-state +       |                    |
|       |       |              | approval stores   |                    |
|       |       |              | [partially implemented]  |                    |
|       |       |              +-------------------+                    |
|       |       |                                                       |
|       |       | spawn_json                                             |
|       |       v                                                       |
|       |  +-------------------+     background execution               |
|       |  | ProcessManager / | ------------------------------------+   |
|       |  | ProcessStore     |                                     |   |
|       |  | [implemented slice]         |                                     |   |
|       |  +---------+--------+                                     |   |
|       |            |                                              |   |
|       |            v                                              |   |
|       |  +---------------------------+                            |   |
|       |  | BackgroundProcessManager  |                            |   |
|       |  | + ProcessExecutor         |                            |   |
|       |  | + DispatchProcessExecutor |                            |   |
|       |  | [implemented slice]                  |                            |   |
|       |  +-------------+-------------+                            |   |
|       |                | uses owned dispatcher handles             |   |
|       |                v                                           |   |
|       |  RuntimeDispatcher::from_arcs(...) [implemented slice]                |   |
|       |                                                            |   |
|       v                                                            |   |
|  +-------------------+                                             |   |
|  | RuntimeDispatcher | <-------------------------------------------+   |
|  | authorized adapter|                                                 |
|  | router only       |                                                 |
|  | [implemented slice]          |                                                 |
|  +----+--------+-----+                                                 |
|       |        |                                                       |
+-------|--------|-------------------------------------------------------+
        |        |
        v        v
+--------------+ +---------------+ +---------------+ +------------------+
| WASM adapter | | Script adapter| | MCP adapter   | | FirstParty/System|
| -> runtime   | | -> runtime    | | -> runtime    | | runtime adapters |
| [implemented slice]     | | [implemented slice]      | | [implemented slice]      | | [not implemented]   |
+--------------+ +---------------+ +---------------+ +------------------+
        ^                ^                 ^
        |                |                 |
+-------+----------------+-----------------+---------------------------+
| ExtensionDiscovery / ExtensionRegistry [implemented slice]                      |
| discovers manifests, packages, capabilities, runtime declarations;    |
| knows what can run, never executes it.                                |
+-----------------------------------------------------------------------+

+-----------------------------------------------------------------------+
| Shared host services and records                                       |
| RootFilesystem/mounts [implemented slice]  ResourceGovernor [implemented slice]              |
| Network policy boundary [implemented slice] Runtime/control-plane events [partially implemented]|
| Process persistence + ProcessHost [implemented slice] Durable leases [partially implemented]    |
| Secret FS durability/leases [implemented slice]                                   |
| User-facing scoped event API [not implemented]                                  |
+-----------------------------------------------------------------------+
```

Key boundary decisions shown above:

- There is **one Host core** with stable ports for transports, runtimes, filesystem, resources, approvals, and events.
- Telegram, Slack, Web, and CLI are **channel adapters/drivers**, not separate hosts.
- Vendor-specific behavior belongs in adapters or extension packages behind the host API, not in duplicated host cores.
- The parent agent loop should be a **first-party hosted service/extension**, not kernel, dispatcher, or transport-driver logic.

---

## 2. Current caller path

The current host-facing invocation path is:

```text
caller / first-party service / future turn service
  -> CapabilityHost::invoke_json(...) | resume_json(...) | spawn_json(...)
      -> validates ExecutionContext and ResourceScope consistency
      -> looks up CapabilityDescriptor in ExtensionRegistry
      -> asks authorizer / approval / lease services for a decision
      -> satisfies authorization obligations through configured handler, or fails closed
      -> records run-state when configured
      -> dispatches only if authorized and obligations are satisfied
      -> either:
           dispatch_json(...) through RuntimeDispatcher
           or create a ProcessRecord through ProcessManager
```

`CapabilityHost` is the caller-facing authority and workflow gate. Callers should not manually evaluate grants and then call `RuntimeDispatcher` as if it were the public workflow API.

`RuntimeDispatcher` is deliberately lower-level:

```text
already-authorized CapabilityDispatchRequest
  -> runtime-kind selection
  -> registered RuntimeAdapter backend
  -> normalized CapabilityDispatchResult
```

The dispatcher does not own authorization, approval semantics, extension discovery, run-state, product workflows, prompt assembly, transport behavior, or concrete WASM/Script/MCP runtime execution. Concrete runtime crates are adapted outside the dispatcher boundary.

---

## 3. Background/process execution path

Process/background execution exists as a capability-backed slice, not as an arbitrary host-process escape hatch.

```text
CapabilityHost::spawn_json(...)
  -> authorize SpawnCapability + target effects
  -> satisfy authorization obligations through configured handler, or fail closed
  -> ProcessManager::spawn(ProcessStart)
  -> ProcessStore persists ProcessRecord as Running
  -> BackgroundProcessManager starts a ProcessExecutor task
  -> DispatchProcessExecutor adapts the process request back into capability dispatch
  -> RuntimeDispatcher::from_arcs(...) provides owned dispatcher composition for detached work
  -> executor success/failure transitions process to Completed/Failed
```

Implemented/current pieces:

- `ProcessRecord` carries `ProcessId`, parent process id, invocation id, tenant/user scope, extension id, capability id, runtime kind, grants, mounts, resource estimate, optional reservation id, and status.
- `ProcessStatus` currently covers `Running`, `Completed`, `Failed`, and `Killed`.
- `BackgroundProcessManager`, `ProcessExecutor`, and `DispatchProcessExecutor` establish the detachable execution seam.
- `RuntimeDispatcher::from_arcs` exists so background execution can hold owned service handles without leaking borrowed request state into spawned tasks.
- Process persistence exists through in-memory and filesystem-backed stores.
- `ProcessHost` exists as the current host-facing `status`, `kill`, `await_process`, `subscribe`, `result`, `output`, and `await_result` API over scoped process current state/results; when wired to `ProcessCancellationRegistry`, scoped kill also signals cooperative executor cancellation.
- `ProcessServices` exists as convenience composition so `ProcessHost` and `BackgroundProcessManager` share the same process store, result store, and cancellation registry.
- `CapabilityHost::with_process_services(...)` exists as convenience spawn wiring that derives the process manager from that shared services bundle without absorbing process lifecycle/result APIs.
- `HostRuntimeServices` exists as a composition-only helper that builds `RuntimeDispatcher`, concrete runtime adapter wrappers, `CapabilityHost`, `ApprovalResolver`, and `ProcessHost` handles from shared registry/filesystem/governor/authorizer/runtime/process/approval/obligation-handler services. Its built-in obligation handler supports metadata-only `AuditBefore` and hands accepted `ApplyNetworkPolicy` obligations to the WASM runtime adapter for host-HTTP enforcement through `ironclaw_network` hardened egress or custom `WasmHostHttp` clients.
- Process lifecycle events exist through `EventingProcessStore` and runtime `EventSink`; approval-resolution audit exists through optional `ApprovalResolver` `AuditSink` wiring and typed `AuditEnvelope::approval_resolved(...)` records.
- Process resource reservation ownership exists through `ResourceManagedProcessStore`; public process starts cannot forge reserved handles, and runtime-backed process dispatch suppresses duplicate reservation through the process-dispatch adapter.

Still missing for process/product completeness:

- productized process event projections/read APIs
- forced/preemptive abort handles for uncooperative executors
- generalized artifact references for large/sensitive/streaming process outputs beyond the current filesystem JSON output path
- durable subscription cursors and event fanout
- dynamic executor-reported process resource usage
- richer process tree/query APIs beyond parent id storage

---

## 4. Implementation status by slice

The current Reborn stack includes these contract and implementation slices. These rows do not claim full product completion unless they explicitly use `[fully implemented]`:

| Area | Current status |
| --- | --- |
| Host API vocabulary | `[implemented slice]` IDs, scopes, runtime kinds, trust classes, capabilities, grants, resources, approvals, events, paths, mount views, neutral dispatch port contracts, and redacted runtime dispatch error kinds |
| Filesystem/mount surface | `[implemented slice]` root/scoped filesystem contracts, `CompositeRootFilesystem`, `FilesystemCatalog`, catalog descriptors/placement metadata, local backend, and feature-gated PostgreSQL/libSQL `RootFilesystem` backends over `root_filesystem_entries`; used by Reborn services through virtual paths while typed repositories remain valid for structured state |
| Memory/workspace filesystem adapter | `[partially implemented]` `ironclaw_memory` owns `/memory/tenants/{tenant}/users/{user}/projects/{project}/...` path grammar, `MemoryBackend` plugin contract, capability declarations, host-resolved `MemoryContext`, `MemoryBackendFilesystemAdapter`, `RepositoryMemoryBackend`, legacy-compatible `MemoryDocumentFilesystem`, repository/indexer seams, in-memory repository, PostgreSQL/libSQL adapters over the existing workspace table family, metadata/.config inheritance, schema validation, skip-indexing/versioning behavior, embedding-provider seam, embedded chunk writes, libSQL/PostgreSQL FTS search, rank-fused full-text/vector search APIs, and a chunking indexer ported from current workspace chunk/hash behavior; production provider credential/network wiring, multi-scope search, and richer provider-specific search result metadata are not complete |
| Extension discovery/registry | `[implemented slice]` manifests, package validation, capability descriptors, runtime declaration mapping |
| Resource governor | `[implemented slice]` reservation/reconcile/release model and V1 dimensions for hosted resource control |
| Secrets | `[partially implemented]` `ironclaw_secrets` service boundary with scoped metadata, AES-256-GCM/HKDF encryption, encrypted-row repository contract, in-memory encrypted repository, filesystem-backed encrypted repository experiment/reference over `RootFilesystem`, credential mapping metadata, and one-shot secret leases; final storage placement is expected to favor typed secret repositories over generic file mounts, and keychain master-key composition/runtime injection are not complete |
| Network | `[partially implemented]` `ironclaw_network` service boundary with scoped policy evaluation, exact/wildcard target matching, literal private IP denial, and egress-estimate checks; HTTP execution/proxying and DNS rebinding protection are not complete |
| Capability access | `[partially implemented]` grant matching, action-time authorization, lease-backed authorizer semantics, in-memory and filesystem-backed exact-invocation lease stores |
| CapabilityHost | `[implemented slice]` caller-facing invocation, approval-blocking, resume, spawn workflow gate, fail-closed `CapabilityObligationHandler` seam, and `ProcessServices` spawn wiring over the neutral host API dispatch port |
| Host runtime composition | `[implemented slice]` `HostRuntimeServices` composition helper for shared registry/filesystem/governor/authorizer/runtime/process/approval/obligation-handler services -> `RuntimeDispatcher`, `CapabilityHost`, `ApprovalResolver`, and `ProcessHost` handles; includes metadata-only built-in `AuditBefore`, WASM-enforced `ApplyNetworkPolicy` obligation handling through hardened network egress or custom host HTTP clients, and optional already-resolved runtime HTTP credential injection after request leak scanning |
| Architecture guardrails | `[partially implemented]` `ironclaw_architecture` test crate walks `cargo metadata` and enforces central Reborn dependency-boundary rules; per-crate guardrail files document local invariants |
| Approvals/resume | `[partially implemented]` pending approval records, invocation fingerprints, approval resolver, fail-closed approval+lease persistence ordering/rollback, metadata-only `AuditEnvelope::approval_resolved(...)` audit records with JSONL persistence coverage, in-memory and async filesystem-backed exact-invocation leases, `resume_json` replay checks |
| Run-state | `[implemented slice]` `Running`, `BlockedApproval`, `BlockedAuth`, `Completed`, `Failed` current-state stores with tenant/user partitioning |
| Dispatcher | `[implemented slice]` implementation of the neutral `ironclaw_host_api` dispatch port for already-authorized requests to registered runtime adapters; no normal dependencies on concrete WASM/Script/MCP runtime crates; missing adapters fail closed before reservation; event sink failures are best-effort and runtime failures are redacted to stable kinds |
| Runtime events and audit | `[partially implemented]` runtime/process `RuntimeEvent` vocabulary with `EventSink`, separate control-plane `AuditEnvelope` records with `AuditSink`, in-memory/JSONL sinks, tenant/user-scoped JSONL path helpers, and hardened read-error semantics; sink failures are ignored by dispatcher/resolver so observability outages do not alter runtime or control-plane outcomes |
| WASM lane | `[implemented slice]` `WasmRuntimeAdapter` composition in `ironclaw_host_runtime` delegates to configured `WasmRuntime` and can enforce accepted `ApplyNetworkPolicy` obligations through `ironclaw_network::HardenedHttpEgressClient` or `WasmPolicyHttpClient` on host-mediated HTTP imports; hardened egress scans guest request/response data and can inject already-resolved HTTP credentials without exposing them to the guest |
| Script lane | `[implemented slice]` `ScriptRuntimeAdapter` composition in `ironclaw_host_runtime` delegates to `ScriptExecutor` with semantic manifest runner profiles, in-process demo backend, and optional legacy Docker backend support |
| MCP lane | `[implemented slice]` `McpRuntimeAdapter` composition in `ironclaw_host_runtime` delegates to `McpExecutor`; not a full MCP lifecycle product yet |
| Process persistence | `[implemented slice]` process store/manager records, scoped process result records with inline JSON or filesystem output refs, `ProcessServices` wiring, host-facing `ProcessHost` status/kill/await/subscribe/result/output APIs, cooperative cancellation tokens, background completion/failure transition protection, lifecycle events, and resource reservation ownership/cleanup |
| Live vertical slice | `[implemented slice]` runnable demos through discovery -> registry -> dispatcher adapters -> resources/events and through `CapabilityHost` -> authorization -> host-runtime-composed dispatcher adapters -> process services; host-runtime composition helper covers shared service wiring and has non-Docker in-memory and filesystem-backed live examples |

---

## 5. What does not exist yet

These are explicit gaps, not architecture contradictions:

| Gap | Why it matters |
| --- | --- |
| Real Telegram/channel adapters | Telegram/Slack/Web/CLI should be transport drivers over the shared host request/event contracts; product-grade channel adapters still need to be built or ported into this shape. |
| Turn service | The shared service that owns one-active-run-per-thread, turn lifecycle, checkpoint/resume edge, and handoff to the loop is not implemented yet. |
| First-party agent loop runtime | The default parent agent loop should be hosted as a first-party service/extension that emits `Reply | CapabilityCalls`; it is not yet a Reborn runtime/service. |
| Process product APIs | Process records, scoped status/kill/await/subscribe/result/output APIs, cooperative cancellation tokens, result records with filesystem JSON output refs, lifecycle events, and resource cleanup ownership exist as service slices; generalized artifact refs for streaming/binary outputs, output streams, forced abort handles, richer scoped read/projection APIs, durable subscription cursors, and event fanout are not complete. |
| Memory plugin/indexer/search wiring | `ironclaw_memory` now owns the memory backend plugin contract and filesystem adapter plus PostgreSQL/libSQL adapters for `memory_documents`, `memory_chunks`/FTS, and `memory_document_versions`, including metadata inheritance/schema validation, skip-indexing/versioning behavior, embedding-provider integration, and rank-fused full-text/vector search APIs; external MCP/WASM/Rust backend adapters, production provider credential/network wiring, multi-scope search, and richer provider-specific search result metadata are not complete. |
| Durable leases | Async filesystem-backed exact-invocation lease persistence now covers issue, claim, consume, revoke, reload, tenant/user/invocation isolation, and fail-closed approval+lease coordination without nested async `block_on`; single-store ACID transactions, full audit retention policy, and reusable approval scopes are not complete. |
| User-facing scoped event API | Runtime/process events, approval audit records, tenant/user-scoped JSONL helpers, and JSONL/in-memory sinks exist, but scoped SSE/WebSocket/reconnect APIs and projection reducers are not productized. |
| Network execution boundary | Scoped network policy evaluation plus hardened runtime HTTP egress now cover DNS/private-address checks, redirect re-validation, pinned resolution, response-size bounds, WASM host-HTTP `ApplyNetworkPolicy` enforcement, host-runtime request/response leak scanning, and optional already-resolved credential injection; product proxying, secret lease consumption, trace recording, non-WASM enforcement, and network egress resource reservation are not complete. |
| FirstParty/System runtime execution | `RuntimeKind::FirstParty` and `RuntimeKind::System` are recognized host API/runtime markers, but no trusted host service adapters are registered yet. |
| Full MCP server lifecycle | MCP is a current adapter lane, not yet a complete product lifecycle for server install/start/auth/restart/monitoring. |
| Auth-blocked resume product path | `BlockedAuth` is reserved in run-state; full OAuth/token prompt, callback, and retry-after-auth workflow remains to be implemented. |
| Concrete obligation handlers | Built-ins now cover metadata-only `AuditBefore` and WASM-enforced `ApplyNetworkPolicy` backed by hardened network egress; already-resolved HTTP credentials can be injected by explicit host-runtime configuration, but `InjectSecretOnce`, `AuditAfter`, `RedactOutput`, `EnforceOutputLimit`, resource reservation, scoped mounts, and non-WASM network enforcement remain fail-closed until required runtime/input/output plumbing exists. |
| Secret injection and durability | Scoped secret metadata, credential mapping metadata, AES-256-GCM/HKDF encryption, encrypted-row repository contract, in-memory encrypted repository, filesystem-backed encrypted repository experiment/reference over `RootFilesystem`, and one-shot leases exist; final PostgreSQL/libSQL durability should be revisited as typed secret repositories outside generic file mounts, and secrets still need keychain master-key resolution, audit emission, rotation, secret lease consumption in obligation handlers, and production `InjectSecretOnce` secret material injection. |

---

## 6. Adapter and host naming rules

Use these naming rules in future docs and implementation plans:

```text
Correct:
  Host core
  Runtime port
  TransportAdapter
  Telegram channel adapter
  Slack channel adapter
  Web gateway adapter
  CLI driver
  first-party agent loop service/extension

Avoid:
  Telegram host
  Slack host
  Web host
  per-vendor host
  dispatcher-owned agent loop
  kernel-owned product workflow
```

The host is the authority envelope. Adapters translate protocol-specific ingress/egress into host requests and events. Runtime lanes execute already-authorized capability work. Product behavior should live as first-party or third-party userland over those contracts.

---

## 7. Agent loop placement

The current architecture decision is:

```text
agent loop = first-party hosted service/extension
agent loop != kernel
agent loop != RuntimeDispatcher
agent loop != transport adapter
```

The loop boundary should stay:

```text
Reply | CapabilityCalls
```

Where:

- `Reply` is user-visible output for the active thread.
- `CapabilityCalls` are explicit capability requests against the visible capability surface.

CodeAct, scripting, subagents, jobs, and other worker modes should be expressed as capabilities such as `spawn_subagent(...)`, `create_job(...)`, or `script.run(...)`, then pass through `CapabilityHost` and the authorized runtime dispatch path.

---

## 8. Source contracts

Use these docs as the detailed contract sources behind this map:

- `docs/reborn/2026-04-25-storage-catalog-and-placement.md`
- `docs/reborn/contracts/host-api.md`
- `docs/reborn/contracts/extensions.md`
- `docs/reborn/contracts/capability-access.md`
- `docs/reborn/contracts/capabilities.md`
- `docs/reborn/contracts/approvals.md`
- `docs/reborn/contracts/run-state.md`
- `docs/reborn/contracts/dispatcher.md`
- `docs/reborn/contracts/processes.md`
- `docs/reborn/contracts/runtime-selection.md`
- `docs/reborn/contracts/runtime-profiles.md`
- `docs/reborn/contracts/resources.md`
- `docs/reborn/contracts/secrets.md`
- `docs/reborn/contracts/network.md`
- `docs/reborn/contracts/events.md`
- `docs/reborn/contracts/events-projections.md`
- `docs/reborn/contracts/agent-loop-protocol.md`
- `docs/reborn/contracts/lightweight-agent-loop.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/live-vertical-slice.md`
