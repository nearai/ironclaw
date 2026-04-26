# IronClaw Reborn — Current architecture map

**Date:** 2026-04-25
**Generated:** 2026-04-25T12:18:38Z
**Status:** Current docs snapshot / implementation-alignment map
**Scope:** Reborn host architecture, current implemented slices, and explicit gaps

This document records the current Reborn shape after the recent architecture discussion. It is a map, not a replacement for the contract docs under `docs/reborn/contracts/`.

Terminology note: older docs use **kernel** for the small host-core composition concept. The current concrete crate for that concept is `ironclaw_host_runtime`; there is no active `ironclaw_kernel` crate in the Reborn stack.

Legend:

```text
[exists]   implemented or covered by current Reborn contract/demo slices
[partial]  present as a narrow slice, not yet a product-complete service
[not yet]  intentionally missing or deferred
```

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
                  | [not yet]                 |
                  +-------------+-------------+
                                |
                                v
                  +---------------------------+
                  | First-party agent loop    |
                  | hosted service/extension  |
                  | emits Reply | Capability  |
                  | Calls [not yet]           |
                  +-------------+-------------+
                                |
                                v
+-------------------------------+---------------------------------------+
|                         HOST CORE                                     |
|                                                                       |
|  +-------------------+       +-------------------+                    |
|  | CapabilityHost    | ----> | Authorization /   |                    |
|  | caller-facing     |       | grants / leases   |                    |
|  | workflow gate     |       | [exists/partial]  |                    |
|  | [exists]          |       +---------+---------+                    |
|  +----+-------+------+                 |                              |
|       |       |                        v                              |
|       |       |              +-------------------+                    |
|       |       |              | Run-state +       |                    |
|       |       |              | approval stores   |                    |
|       |       |              | [exists/partial]  |                    |
|       |       |              +-------------------+                    |
|       |       |                                                       |
|       |       | spawn_json                                             |
|       |       v                                                       |
|       |  +-------------------+     background execution               |
|       |  | ProcessManager / | ------------------------------------+   |
|       |  | ProcessStore     |                                     |   |
|       |  | [exists]         |                                     |   |
|       |  +---------+--------+                                     |   |
|       |            |                                              |   |
|       |            v                                              |   |
|       |  +---------------------------+                            |   |
|       |  | BackgroundProcessManager  |                            |   |
|       |  | + ProcessExecutor         |                            |   |
|       |  | + DispatchProcessExecutor |                            |   |
|       |  | [exists]                  |                            |   |
|       |  +-------------+-------------+                            |   |
|       |                | uses owned dispatcher handles             |   |
|       |                v                                           |   |
|       |  RuntimeDispatcher::from_arcs(...) [exists]                |   |
|       |                                                            |   |
|       v                                                            |   |
|  +-------------------+                                             |   |
|  | RuntimeDispatcher | <-------------------------------------------+   |
|  | authorized adapter|                                                 |
|  | router only       |                                                 |
|  | [exists]          |                                                 |
|  +----+--------+-----+                                                 |
|       |        |                                                       |
+-------|--------|-------------------------------------------------------+
        |        |
        v        v
+--------------+ +---------------+ +---------------+ +------------------+
| WASM adapter | | Script adapter| | MCP adapter   | | FirstParty/System|
| -> runtime   | | -> runtime    | | -> runtime    | | runtime adapters |
| [exists]     | | [exists]      | | [exists]      | | [not yet exec]   |
+--------------+ +---------------+ +---------------+ +------------------+
        ^                ^                 ^
        |                |                 |
+-------+----------------+-----------------+---------------------------+
| ExtensionDiscovery / ExtensionRegistry [exists]                      |
| discovers manifests, packages, capabilities, runtime declarations;    |
| knows what can run, never executes it.                                |
+-----------------------------------------------------------------------+

+-----------------------------------------------------------------------+
| Shared host services and records                                       |
| RootFilesystem/mounts [exists]  ResourceGovernor [exists]              |
| Network policy boundary [exists] Runtime/control-plane events [partial]|
| Process persistence + ProcessHost [exists] Durable leases [partial]    |
| Secret metadata/leases [exists]                                        |
| User-facing scoped event API [not yet]                                  |
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
- `HostRuntimeServices` exists as a composition-only helper that builds `RuntimeDispatcher`, concrete runtime adapter wrappers, `CapabilityHost`, `ApprovalResolver`, and `ProcessHost` handles from shared registry/filesystem/governor/authorizer/runtime/process/approval/obligation-handler services. Its built-in obligation handler currently supports metadata-only `AuditBefore` and `ApplyNetworkPolicy` preflight.
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

## 4. What exists now

The current implemented or contract-backed Reborn stack includes these slices:

| Area | Current status |
| --- | --- |
| Host API vocabulary | `[exists]` IDs, scopes, runtime kinds, trust classes, capabilities, grants, resources, approvals, events, paths, mount views, neutral dispatch port contracts, and redacted runtime dispatch error kinds |
| Filesystem/mount surface | `[exists]` root/scoped filesystem contracts and filesystem-backed stores used by Reborn services |
| Extension discovery/registry | `[exists]` manifests, package validation, capability descriptors, runtime declaration mapping |
| Resource governor | `[exists]` reservation/reconcile/release model and V1 dimensions for hosted resource control |
| Secrets | `[exists/partial]` `ironclaw_secrets` service boundary with scoped metadata, in-memory storage, and one-shot secret leases; durable encrypted persistence and runtime injection are not complete |
| Network | `[exists/partial]` `ironclaw_network` service boundary with scoped policy evaluation, exact/wildcard target matching, literal private IP denial, and egress-estimate checks; HTTP execution/proxying and DNS rebinding protection are not complete |
| Capability access | `[exists/partial]` grant matching, action-time authorization, lease-backed authorizer semantics, in-memory and filesystem-backed exact-invocation lease stores |
| CapabilityHost | `[exists]` caller-facing invocation, approval-blocking, resume, spawn workflow gate, fail-closed `CapabilityObligationHandler` seam, and `ProcessServices` spawn wiring over the neutral host API dispatch port |
| Host runtime composition | `[exists]` `HostRuntimeServices` composition helper for shared registry/filesystem/governor/authorizer/runtime/process/approval/obligation-handler services -> `RuntimeDispatcher`, `CapabilityHost`, `ApprovalResolver`, and `ProcessHost` handles; includes metadata-only built-in `AuditBefore` and `ApplyNetworkPolicy` obligation handler |
| Architecture guardrails | `[exists/partial]` `ironclaw_architecture` test crate walks `cargo metadata` and enforces central Reborn dependency-boundary rules; per-crate guardrail files document local invariants |
| Approvals/resume | `[exists/partial]` pending approval records, invocation fingerprints, approval resolver, fail-closed approval+lease persistence ordering/rollback, metadata-only `AuditEnvelope::approval_resolved(...)` audit records with JSONL persistence coverage, in-memory and async filesystem-backed exact-invocation leases, `resume_json` replay checks |
| Run-state | `[exists]` `Running`, `BlockedApproval`, `BlockedAuth`, `Completed`, `Failed` current-state stores with tenant/user partitioning |
| Dispatcher | `[exists]` implementation of the neutral `ironclaw_host_api` dispatch port for already-authorized requests to registered runtime adapters; no normal dependencies on concrete WASM/Script/MCP runtime crates; missing adapters fail closed before reservation; event sink failures are best-effort and runtime failures are redacted to stable kinds |
| Runtime events and audit | `[partial]` runtime/process `RuntimeEvent` vocabulary with `EventSink`, separate control-plane `AuditEnvelope` records with `AuditSink`, in-memory/JSONL sinks, tenant/user-scoped JSONL path helpers, and hardened read-error semantics; sink failures are ignored by dispatcher/resolver so observability outages do not alter runtime or control-plane outcomes |
| WASM lane | `[exists]` `WasmRuntimeAdapter` composition in `ironclaw_host_runtime` delegates to configured `WasmRuntime` |
| Script lane | `[exists]` `ScriptRuntimeAdapter` composition in `ironclaw_host_runtime` delegates to `ScriptExecutor` with semantic manifest runner profiles, in-process demo backend, and optional legacy Docker backend support |
| MCP lane | `[exists]` `McpRuntimeAdapter` composition in `ironclaw_host_runtime` delegates to `McpExecutor`; not a full MCP lifecycle product yet |
| Process persistence | `[exists]` process store/manager records, scoped process result records with inline JSON or filesystem output refs, `ProcessServices` wiring, host-facing `ProcessHost` status/kill/await/subscribe/result/output APIs, cooperative cancellation tokens, background completion/failure transition protection, lifecycle events, and resource reservation ownership/cleanup |
| Live vertical slice | `[exists]` runnable demos through discovery -> registry -> dispatcher adapters -> resources/events and through `CapabilityHost` -> authorization -> host-runtime-composed dispatcher adapters -> process services; host-runtime composition helper covers shared service wiring and has non-Docker in-memory and filesystem-backed live examples |

---

## 5. What does not exist yet

These are explicit gaps, not architecture contradictions:

| Gap | Why it matters |
| --- | --- |
| Real Telegram/channel adapters | Telegram/Slack/Web/CLI should be transport drivers over the shared host request/event contracts; product-grade channel adapters still need to be built or ported into this shape. |
| Turn service | The shared service that owns one-active-run-per-thread, turn lifecycle, checkpoint/resume edge, and handoff to the loop is not implemented yet. |
| First-party agent loop runtime | The default parent agent loop should be hosted as a first-party service/extension that emits `Reply | CapabilityCalls`; it is not yet a Reborn runtime/service. |
| Process product APIs | Process records, scoped status/kill/await/subscribe/result/output APIs, cooperative cancellation tokens, result records with filesystem JSON output refs, lifecycle events, and resource cleanup ownership exist as service slices; generalized artifact refs for streaming/binary outputs, output streams, forced abort handles, richer scoped read/projection APIs, durable subscription cursors, and event fanout are not complete. |
| Durable leases | Async filesystem-backed exact-invocation lease persistence now covers issue, claim, consume, revoke, reload, tenant/user/invocation isolation, and fail-closed approval+lease coordination without nested async `block_on`; single-store ACID transactions, full audit retention policy, and reusable approval scopes are not complete. |
| User-facing scoped event API | Runtime/process events, approval audit records, tenant/user-scoped JSONL helpers, and JSONL/in-memory sinks exist, but scoped SSE/WebSocket/reconnect APIs and projection reducers are not productized. |
| Network execution boundary | Scoped network policy evaluation and metadata-only `ApplyNetworkPolicy` obligation preflight exist, but HTTP client/proxy execution, DNS resolution/rebinding defenses, response streaming, and network egress resource reservation are not complete. |
| FirstParty/System runtime execution | `RuntimeKind::FirstParty` and `RuntimeKind::System` are recognized host API/runtime markers, but no trusted host service adapters are registered yet. |
| Full MCP server lifecycle | MCP is a current adapter lane, not yet a complete product lifecycle for server install/start/auth/restart/monitoring. |
| Auth-blocked resume product path | `BlockedAuth` is reserved in run-state; full OAuth/token prompt, callback, and retry-after-auth workflow remains to be implemented. |
| Concrete obligation handlers | Metadata-only built-ins now cover `AuditBefore` and `ApplyNetworkPolicy`; built-ins for `InjectSecretOnce`, `AuditAfter`, `RedactOutput`, `EnforceOutputLimit`, resource reservation, and scoped mounts remain fail-closed until required runtime/input/output plumbing exists. |
| Secret injection and durability | Scoped in-memory secret metadata and one-shot leases exist; encrypted durable persistence, audit emission, rotation, and production `InjectSecretOnce` secret material injection remain to be implemented. |

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
