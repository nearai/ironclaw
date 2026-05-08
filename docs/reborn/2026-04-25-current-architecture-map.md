# IronClaw Reborn — Current architecture map

**Date:** 2026-04-25
**Generated:** 2026-04-25T12:18:38Z
**Last updated:** 2026-04-27 after contract-freeze review updates and PR1 landing-plan carve-out
**Status:** Current docs snapshot / implementation-alignment map
**Scope:** Reborn host architecture, current implemented slices, and explicit gaps

This document records the current Reborn shape after the recent architecture discussion. It is a map, not a replacement for the contract docs under `docs/reborn/contracts/`.

Terminology note: **kernel** is the architectural security perimeter: the small set of mediated services that enforce authority, isolation, leases, obligations, redaction, resources, scoped storage, network policy, and durable audit/event substrate. The current concrete composition crate for kernel-facing services is `ironclaw_host_runtime`; there is no active `ironclaw_kernel` crate in the Reborn stack.

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
docs/reborn/contracts/kernel-boundary.md
docs/reborn/contracts/storage-placement.md
docs/reborn/contracts/memory.md
docs/reborn/contracts/settings-config.md
docs/reborn/contracts/turns-agent-loop.md
docs/reborn/contracts/migration-compatibility.md
```

Current landing tracker:

```text
https://github.com/nearai/ironclaw/issues/2987
```

These docs record the delegation-ready system decisions: kernel as security perimeter, loops/userland running on the kernel surface, first-class optional `AgentId`, hybrid storage placement, typed repositories for structured state, split memory services over shared backends, durable event streams with replay cursors, all built-in obligations for V1, all three runtime lanes as first-class, and schema reuse where viable.

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
                  | Turn coordination /       |
                  | reference or custom loop  |
                  | one active run/thread,    |
                  | emits Reply | Capability  |
                  | Calls [not implemented]   |
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

- There is **one kernel/host core** with stable ports for transports, runtimes, filesystem, resources, approvals, and events.
- The kernel is the security perimeter, not the agent brain; it is defined by what it mediates and secures.
- Telegram, Slack, Web, and CLI are **channel adapters/drivers**, not separate hosts.
- Vendor-specific behavior belongs in adapters or extension packages behind the host API, not in duplicated host cores.
- The parent agent loop is **userland running on the kernel surface**, not kernel, dispatcher, or transport-driver logic. Shipped reference loops still need explicit grants and kernel-mediated calls.

---

## 2. Current caller path

The current host-facing invocation path is:

```text
caller / shipped reference loop / future custom loop / turn coordinator
  -> CapabilityHost::invoke_json(...) | resume_json(...) | spawn_json(...)
      -> validates ExecutionContext and ResourceScope consistency
      -> looks up CapabilityDescriptor in ExtensionRegistry
      -> asks authorizer / approval / lease services for a decision
      -> prepares authorization obligations through configured handler, or fails closed
      -> records run-state when configured
      -> dispatches only if authorized and obligations are prepared/satisfied
      -> completes post-dispatch obligations before returning immediate output
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
  -> prepare authorization obligations through configured handler, or fail closed
  -> ProcessManager::spawn(ProcessStart)
  -> ProcessStore persists ProcessRecord as Running
  -> BackgroundProcessManager starts a ProcessExecutor task
  -> DispatchProcessExecutor adapts the process request back into capability dispatch
  -> RuntimeDispatcher::from_arcs(...) provides owned dispatcher composition for detached work
  -> executor success/failure transitions process to Completed/Failed
```

Implemented/current pieces:

- `ProcessRecord` carries `ProcessId`, parent process id, invocation id, tenant/user/project/agent scope where available through `ResourceScope`, extension id, capability id, runtime kind, grants, mounts, resource estimate, optional reservation id, and status.
- `ProcessStatus` currently covers `Running`, `Completed`, `Failed`, and `Killed`.
- `BackgroundProcessManager`, `ProcessExecutor`, and `DispatchProcessExecutor` establish the detachable execution seam.
- `RuntimeDispatcher::from_arcs` exists so background execution can hold owned service handles without leaking borrowed request state into spawned tasks.
- Process persistence exists through in-memory and filesystem-backed stores.
- `ProcessHost` exists as the current host-facing `status`, `kill`, `await_process`, `subscribe`, `result`, `output`, and `await_result` API over scoped process current state/results; when wired to `ProcessCancellationRegistry`, scoped kill also signals cooperative executor cancellation.
- `ProcessServices` exists as convenience composition so `ProcessHost` and `BackgroundProcessManager` share the same process store, result store, and cancellation registry.
- `CapabilityHost::with_process_services(...)` exists as convenience spawn wiring that derives the process manager from that shared services bundle without absorbing process lifecycle/result APIs.
- `HostRuntimeServices` exists as a composition-only helper that builds `RuntimeDispatcher`, concrete runtime adapter wrappers, `CapabilityHost`, `ApprovalResolver`, and `ProcessHost` handles from shared registry/filesystem/governor/authorizer/runtime/process/approval/obligation-handler services. Its built-in obligation handler now supports the V1 immediate dispatch/resume obligation set: `AuditBefore`, `AuditAfter`, `ApplyNetworkPolicy`, direct-handle `InjectSecretOnce`, `RedactOutput`, `EnforceOutputLimit`, `UseScopedMounts`, and immediate `ReserveResources` handoff. Spawn/background resource ownership remains process-store-managed.
- Process lifecycle events exist through `EventingProcessStore` and runtime `EventSink`; approval-resolution audit exists through optional `ApprovalResolver` `AuditSink` wiring and typed `AuditEnvelope::approval_resolved(...)` records.
- Process resource reservation ownership exists through `ResourceManagedProcessStore`; public process starts cannot forge reserved handles, and runtime-backed process dispatch suppresses duplicate reservation through the process-dispatch adapter. Prepared resource reservations from capability obligations are immediate dispatch/resume only and are rejected for spawn to avoid leaks or double ownership.

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
| Filesystem/mount surface | `[implemented slice]` root/scoped filesystem contracts, V1 mutation ops (`append_file`, `delete`, `create_dir_all`), `CompositeRootFilesystem`, `FilesystemCatalog`, catalog descriptors/placement metadata, local backend, and feature-gated PostgreSQL/libSQL `RootFilesystem` backends over `root_filesystem_entries`; used by Reborn services through virtual paths while typed repositories remain valid for structured state |
| Memory/workspace filesystem adapter | `[partially implemented]` `ironclaw_memory` owns `/memory/tenants/{tenant}/users/{user}/agents/{agent-or-_none}/projects/{project-or-_none}/...` path grammar, `MemoryBackend` plugin contract/support declarations, host-resolved `MemoryContext`, `MemoryBackendFilesystemAdapter`, `RepositoryMemoryBackend`, legacy-compatible `MemoryDocumentFilesystem`, repository/indexer seams, in-memory repository, PostgreSQL/libSQL adapters over the existing workspace table family, metadata/.config inheritance, schema validation, skip-indexing/versioning behavior, embedding-provider seam, embedded chunk writes, libSQL/PostgreSQL FTS search, rank-fused full-text/vector search APIs, and a chunking indexer ported from current workspace chunk/hash behavior; production service facades, prompt-safety hooks, provider credential/network wiring, multi-scope prompt parity, and richer provider-specific search result metadata are not complete |
| Extension discovery/registry | `[implemented slice]` manifests, package validation, capability descriptors, runtime declaration mapping |
| Resource governor | `[implemented slice]` reservation/reconcile/release model, agent-scoped account limits, `reserve_with_id(...)` for obligation handoff, duplicate/mismatch reservation errors, and V1 dimensions for hosted resource control |
| Secrets | `[partially implemented]` `ironclaw_secrets` service boundary with scoped metadata, AES-256-GCM/HKDF encryption, encrypted-row repository contract, in-memory encrypted repository, filesystem-backed encrypted repository experiment/reference over `RootFilesystem`, credential mapping metadata, credential-account metadata records, agent/project-scoped secret path partitioning, and one-shot secret leases; production DB-backed typed repository wiring, keychain master-key composition, rotation, OAuth repair, and credential-account-shaped injection are not complete |
| Network | `[partially implemented]` `ironclaw_network` service boundary with scoped policy evaluation, exact/wildcard target matching, literal private IP denial, egress-estimate checks, hardened WASM host-HTTP egress with DNS/private-address checks, redirect re-validation, pinned resolution, response-size bounds, leak scanning, optional already-resolved credential injection, and sanitized stable errors; product proxying, trace recording, non-WASM enforcement, and network egress resource reservation are not complete |
| Capability access | `[partially implemented]` grant matching, action-time authorization, lease-backed authorizer semantics, in-memory and filesystem-backed exact-invocation lease stores |
| CapabilityHost | `[implemented slice]` caller-facing invocation, approval-blocking, resume, spawn workflow gate, fail-closed `CapabilityObligationHandler` seam with prepare/complete/abort phases, prepared mount/resource handoff for immediate dispatch/resume, post-dispatch completion before immediate output return, and `ProcessServices` spawn wiring over the neutral host API dispatch port |
| Host runtime composition | `[implemented slice]` `HostRuntimeServices` composition helper for shared registry/filesystem/governor/authorizer/runtime/process/approval/obligation-handler services -> `RuntimeDispatcher`, `CapabilityHost`, `ApprovalResolver`, and `ProcessHost` handles; built-in obligation handler covers V1 immediate dispatch/resume obligations (`AuditBefore`, `AuditAfter`, `ApplyNetworkPolicy`, direct `InjectSecretOnce`, `RedactOutput`, `EnforceOutputLimit`, `UseScopedMounts`, and `ReserveResources`) plus WASM network-policy handoff through hardened egress/custom host HTTP clients and optional already-resolved runtime HTTP credential injection after request leak scanning |
| Architecture guardrails | `[partially implemented]` `ironclaw_architecture` test crate walks `cargo metadata` and enforces central Reborn dependency-boundary rules; per-crate guardrail files document local invariants |
| Approvals/resume | `[partially implemented]` pending approval records, invocation fingerprints, approval resolver, fail-closed approval+lease persistence ordering/rollback, metadata-only `AuditEnvelope::approval_resolved(...)` audit records with JSONL persistence coverage, in-memory and async filesystem-backed exact-invocation leases, `resume_json` replay checks |
| Run-state | `[implemented slice]` `Running`, `BlockedApproval`, `BlockedAuth`, `Completed`, `Failed` current-state stores with tenant/user partitioning |
| Dispatcher | `[implemented slice]` implementation of the neutral `ironclaw_host_api` dispatch port for already-authorized requests to registered runtime adapters; no normal dependencies on concrete WASM/Script/MCP runtime crates; missing adapters fail closed before reservation; event sink failures are best-effort and runtime failures are redacted to stable kinds |
| Runtime events and audit | `[partially implemented]` runtime/process `RuntimeEvent` vocabulary with `EventSink`, separate control-plane `AuditEnvelope` records with `AuditSink`, in-memory/JSONL sinks, tenant/user/agent-scoped JSONL path helpers, append-only writes, `EventCursor`/`EventReplay` replay after cursor, malformed-log fail-closed behavior, and hardened read-error semantics; sink failures are ignored by dispatcher/resolver so observability outages do not alter runtime or control-plane outcomes |
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
| Kernel turn coordination | The shared kernel-mediated service that owns one-active-run-per-thread, scope-consistent turn/run state, blocked-state coordination, checkpoint/resume edge, and handoff to the loop is not implemented yet. |
| Reference/userland loop runtime | The default parent agent loop should be a shipped reference loop running on the kernel surface and emitting `Reply | CapabilityCalls`; custom loop families are expected later. It is not yet a Reborn runtime/service. |
| Process product APIs | Process records, scoped status/kill/await/subscribe/result/output APIs, cooperative cancellation tokens, result records with filesystem JSON output refs, lifecycle events, and resource cleanup ownership exist as service slices; generalized artifact refs for streaming/binary outputs, output streams, forced abort handles, richer scoped read/projection APIs, durable subscription cursors, and event fanout are not complete. |
| Memory plugin/indexer/search wiring | `ironclaw_memory` now owns the memory backend plugin contract and filesystem adapter plus PostgreSQL/libSQL adapters for `memory_documents`, `memory_chunks`/FTS, and `memory_document_versions`, including metadata inheritance/schema validation, skip-indexing/versioning behavior, embedding-provider integration, and rank-fused full-text/vector search APIs; external MCP/WASM/Rust backend adapters, production provider credential/network wiring, multi-scope search, and richer provider-specific search result metadata are not complete. |
| Durable leases | Async filesystem-backed exact-invocation lease persistence now covers issue, claim, consume, revoke, reload, tenant/user/invocation isolation, and fail-closed approval+lease coordination without nested async `block_on`; single-store ACID transactions, full audit retention policy, and reusable approval scopes are not complete. |
| User-facing scoped event API | Runtime/process events, approval audit records, tenant/user/agent-scoped JSONL helpers, replay cursors, and JSONL/in-memory sinks exist as substrate, but scoped stream envelopes, replay-gap reporting, SSE/WebSocket/reconnect APIs, and projection reducers are not productized. |
| Network execution boundary | Scoped network policy evaluation plus hardened runtime HTTP egress now cover DNS/private-address checks, redirect re-validation, pinned resolution, response-size bounds, WASM host-HTTP `ApplyNetworkPolicy` enforcement, host-runtime request/response leak scanning, and optional already-resolved credential injection; product proxying, secret lease consumption, trace recording, non-WASM enforcement, and network egress resource reservation are not complete. |
| FirstParty/System runtime execution | `RuntimeKind::FirstParty` and `RuntimeKind::System` are recognized host API/runtime markers, but no host-policy-selected service adapters are registered yet. |
| Full MCP server lifecycle | MCP is a current adapter lane, not yet a complete product lifecycle for server install/start/auth/restart/monitoring. |
| Auth-blocked resume product path | `BlockedAuth` is reserved in run-state; full OAuth/token prompt, callback, and retry-after-auth workflow remains to be implemented. |
| Obligation product gaps | Built-ins cover the V1 immediate dispatch/resume set, including direct `InjectSecretOnce`, post-dispatch redaction/output limits/audit-after, scoped mounts, and immediate resource handoff. Remaining gaps are `InjectCredentialOnce`, spawned/background post-output redaction/limits/audit-after, generic runtime environment injection, non-WASM network enforcement, and productized credential-account resolution. |
| Secret injection and durability | Scoped secret metadata, credential mapping/account metadata, AES-256-GCM/HKDF encryption, encrypted-row repository contract, in-memory encrypted repository, filesystem-backed encrypted repository experiment/reference over `RootFilesystem`, one-shot leases, and direct `InjectSecretOnce` obligation staging exist; final PostgreSQL/libSQL durability should use typed secret repositories outside generic file mounts, and secrets still need keychain master-key resolution, audit retention policy, rotation, OAuth repair, and credential-account-based injection. |

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
  shipped reference loop / configured userland loop

Avoid:
  Telegram host
  Slack host
  Web host
  per-vendor host
  dispatcher-owned agent loop
  kernel-owned product workflow
```

The host/kernel is the authority envelope. Adapters translate protocol-specific ingress/egress into host requests and events. Runtime lanes execute already-authorized capability work. Product behavior should live as shipped or third-party userland over those contracts, not inside kernel-owned workflow.

---

## 7. Agent loop placement

The current architecture decision is:

```text
agent loop = userland loop running on the kernel surface
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

CodeAct, scripting, subagents, jobs, and other worker modes should be expressed as userland loop behavior and/or capabilities such as `spawn_subagent(...)`, `create_job(...)`, or `script.run(...)`, then pass through `CapabilityHost` and the authorized runtime dispatch path for privileged effects. A shipped loop may have a higher trust ceiling by host policy, but trust class is not a grant and never allows bypassing the kernel surface.

---

## 8. Source contracts

Use these docs as the detailed contract sources behind this map:

- `docs/reborn/2026-04-25-storage-catalog-and-placement.md`
- `docs/reborn/contracts/host-api.md`
- `docs/reborn/contracts/kernel-boundary.md`
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
