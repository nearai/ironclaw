# IronClaw Reborn host runtime composition contract

**Date:** 2026-04-25
**Status:** V1 composition slice
**Crate:** `crates/ironclaw_host_runtime`
**Depends on:** `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/capability-access.md`

---

## 1. Purpose

`ironclaw_host_runtime` is a composition-only crate for the current Reborn host/runtime vertical slice.

Terminology note: **kernel** is the architectural security perimeter described in [`kernel-boundary.md`](kernel-boundary.md). In the current implementation, `ironclaw_host_runtime` is the concrete composition crate for kernel-facing services/adapters; it is not permission to grow product workflow inside the kernel. There is no active `ironclaw_kernel` crate in the Reborn stack unless a deliberate rename happens later.

It wires already-owned service crates together so application setup code does not repeatedly hand-connect the same shared handles:

```text
ExtensionRegistry
RootFilesystem
ResourceGovernor
CapabilityDispatchAuthorizer
RunStateStore / ApprovalRequestStore / CapabilityLeaseStore
ProcessServices
WASM / Script / MCP runtime backends
EventSink / AuditSink
CapabilityObligationHandler
  -> HostRuntimeServices
      -> WASM / Script / MCP RuntimeAdapter wrappers
      -> RuntimeDispatcher
      -> CapabilityHost
      -> ApprovalResolver
      -> ProcessHost
```

This crate does not define new authority semantics and does not own lifecycle state. It is a narrow composition root over existing contracts.

---

## 2. Boundary

`HostRuntimeServices` may build:

```rust
RuntimeDispatcher<'static, F, G>
Arc<RuntimeDispatcher<'static, F, G>>
CapabilityHost<'_, D>
ProcessHost<'_>
ApprovalResolver<'_, dyn ApprovalRequestStore, dyn CapabilityLeaseStore>
```

It may hold shared `Arc` handles to configured services, runtime backends, observability sinks, an optional WASM host HTTP client or hardened network egress client, optional already-resolved runtime HTTP credentials, an optional scoped `SecretStore`, a one-shot runtime secret injection store, and an optional capability-obligation handler. It adapts concrete runtime crates into `ironclaw_dispatcher::RuntimeAdapter` implementations when building `RuntimeDispatcher`, and it provides a small `BuiltinObligationHandler` for metadata-only audit, scoped network-policy handoff, and direct-handle secret lease consumption.

It must not:

- implement grant matching or spawn policy
- execute runtime lanes directly outside adapter wrappers
- own process state transitions or cancellation semantics
- own approval resolution or lease semantics; `approval_resolver()` only wires `ironclaw_approvals::ApprovalResolver`
- own broad runtime/input/output obligation semantics; `with_obligation_handler(...)` passes a shared `CapabilityObligationHandler` through to `CapabilityHost`, and `BuiltinObligationHandler` is limited to metadata-only audit, scoped mount/resource handoff, `ApplyNetworkPolicy` preflight/hand-off to WASM host HTTP imports and `ironclaw_network` egress, direct `InjectSecretOnce { handle }` lease/consume into a one-shot runtime injection store, and immediate dispatch/resume output redaction/limits. Already-resolved runtime HTTP credentials may be injected only in the hardened WASM egress adapter after pre-injection leak scanning.
- expose process lifecycle APIs through `CapabilityHost`
- turn process subscriptions into a message bus
- weaken tenant/user/agent/project scoped persistence boundaries

Ownership remains:

```text
authorization -> grant, lease, and spawn decisions
capabilities  -> caller-facing invoke/resume/spawn workflow
processes     -> lifecycle, result, output, cancellation, and process services
dispatcher    -> already-authorized runtime routing through registered adapters
host_runtime  -> composition of concrete WASM / Script / MCP adapters
runtimes      -> WASM / Script / MCP execution
```

---

## 3. API shape

The helper is intentionally small:

```rust
let services = HostRuntimeServices::new(
    registry,
    filesystem,
    governor,
    authorizer,
    process_services,
)
.with_run_state(run_state)
.with_approval_requests(approval_requests)
.with_capability_leases(capability_leases)
.with_script_runtime(script_runtime)
.with_wasm_http_client(wasm_http_client)
// or install the built-in hardened Reborn network egress client:
.with_hardened_network_egress()
// optional already-resolved credentials for host-mediated WASM HTTP egress:
.with_runtime_http_credentials(runtime_http_credentials)
// optional direct-handle InjectSecretOnce support:
.with_secret_store(secret_store)
.with_event_sink(event_sink)
.with_audit_sink(audit_sink)
.with_builtin_obligation_handler();

// Or provide a custom handler:
let services = services.with_obligation_handler(obligation_handler);

let dispatcher = services.runtime_dispatcher_arc();
let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);
let approval_resolver = services.approval_resolver();
let process_host = services.process_host();
```

For tests or custom process executors, callers can also provide an arbitrary dispatcher and executor:

```rust
let capability_host = services.capability_host(&dispatcher, executor);
```

`capability_host_for_runtime_dispatcher(...)` derives a `DispatchProcessExecutor` from the same runtime dispatcher used for immediate dispatch. Spawned capability-backed process work therefore routes through the authorized dispatch interface after `CapabilityHost::spawn_json(...)` has authorized, satisfied configured obligations, and recorded the process start. The concrete WASM, Script, and MCP runtimes are wrapped by `WasmRuntimeAdapter`, `ScriptRuntimeAdapter`, and `McpRuntimeAdapter` here instead of being hardcoded into `ironclaw_dispatcher`.

---

## 4. Tenant and lifecycle invariants

The helper preserves the same service handles for `CapabilityHost`, `ApprovalResolver`, and `ProcessHost`:

```text
CapabilityHost::spawn_json(...)
  -> ProcessServices::background_manager(...)
      -> shared ProcessStore
      -> shared ProcessResultStore
      -> shared ProcessCancellationRegistry

ProcessHost status/kill/result/output
  -> ProcessServices::host()
      -> same shared ProcessStore
      -> same shared ProcessResultStore
      -> same shared ProcessCancellationRegistry
```

`approval_resolver()` uses the same `ApprovalRequestStore`, `CapabilityLeaseStore`, and optional `AuditSink` handles configured for `CapabilityHost::resume_json(...)`. This prevents accidental split wiring where one component approves a request into one lease store while resume checks another, or where approval audit disappears because the resolver was not wired to the shared audit sink.

If configured, the shared `CapabilityObligationHandler` is passed into each `CapabilityHost` built by this helper. `HostRuntimeServices::with_builtin_obligation_handler()` installs the built-in handler with the shared `NetworkObligationPolicyStore`, `RuntimeSecretInjectionStore`, and `ResourceGovernor`.

Supported built-in obligations in this slice:

- `AuditBefore`: emits a redacted `AuditStage::Before` audit envelope through the configured `AuditSink`; without an audit sink, it fails closed.
- `ApplyNetworkPolicy`: validates policy metadata and stores the scoped policy in `NetworkObligationPolicyStore`; without a policy store, it fails closed. The policy key includes tenant/user/agent/project/mission/thread/invocation/capability scope.
- `InjectSecretOnce { handle }`: requires `with_secret_store(...)`, verifies the scoped secret exists, calls `SecretStore::lease_once(...)`, calls `SecretStore::consume(...)` exactly once, and stages the returned material in `RuntimeSecretInjectionStore`. Runtime consumers must call `take(...)`, which removes the material so it cannot be reused. Missing secret stores, missing scoped secrets, lease failures, consume failures, and injection-store failures all map to sanitized `CapabilityObligationFailureKind::Secret`.
- `UseScopedMounts { mounts }`: validates the requested mount view and requires it to be a subset of `ExecutionContext.mounts`; the prepared mount view is handed back to `CapabilityHost` for dispatch/process-start attenuation. Broader/duplicate/invalid mount obligations fail closed as `CapabilityObligationFailureKind::Mount`.
- `ReserveResources { reservation_id }`: for immediate dispatch/resume, reserves `CapabilityObligationRequest.estimate` in `ExecutionContext.resource_scope` using the exact requested `ResourceReservationId`. The prepared reservation is handed to `CapabilityHost` and then to `RuntimeDispatcher`/runtime adapters so WASM, Script, and MCP lanes reconcile/release the same reservation instead of double-reserving. If preparation is aborted before ownership transfers, the handler releases the reservation. Missing/denied/mismatched resource handling maps to sanitized `CapabilityObligationFailureKind::Resource`. Spawn keeps resource ownership in `ironclaw_processes::ResourceManagedProcessStore` and rejects prepared resource reservations to avoid leaks or double ownership.
- `AuditAfter`: runs after successful immediate dispatch/resume and emits a redacted `AuditStage::After` audit envelope with metadata-only output byte count.
- `RedactOutput`: runs after successful immediate dispatch/resume and applies `ironclaw_safety::LeakDetector` string redaction recursively to JSON output before callers receive it. Blocking leak detections fail closed as `CapabilityObligationFailureKind::Output`.
- `EnforceOutputLimit { bytes }`: runs after redaction and before returning immediate dispatch/resume output; oversized output fails closed as `CapabilityObligationFailureKind::Output`.

The WASM runtime adapter consumes accepted network policy during dispatch. If `with_hardened_network_egress()` or `with_http_egress_client(...)` is configured, WASM `host.http_request_utf8` calls go through `ironclaw_network::HardenedHttpEgressClient`/`HttpEgressClient`, which policy-checks, DNS-resolves, private-address-checks, redirect-revalidates, pins validated resolution, and bounds response size before returning bytes. The hardened WASM egress adapter also runs `ironclaw_safety::LeakDetector` on the guest-provided URL/body before any credential injection, injects only already-resolved `RuntimeHttpCredential` values configured by the composition layer, and scans/redacts-or-blocks response bodies before returning them to guest memory. If only a custom `WasmHostHttp` is configured, the adapter still wraps it with `WasmPolicyHttpClient` as a lower-level test/custom-client policy guard. If a network policy is present for Script or MCP lanes, those adapters fail closed with `NetworkDenied` until they have enforceable network plumbing. This handoff still does not perform OAuth refresh, account selection, credential-account resolution, or reserve egress resources by itself. Resource-reservation handoff is supported for immediate WASM, Script, and MCP dispatch/resume; dispatcher validation failures release prepared reservations before returning, and runtime adapters reconcile/release prepared reservations instead of double-reserving. Post-output obligations are only supported for immediate dispatch/resume; spawn rejects `AuditAfter`, `RedactOutput`, and `EnforceOutputLimit` because process output completion belongs to process/result services. Unsupported or failed handler outcomes remain fail-closed inside `CapabilityHost` before dispatch, process start, approval lease claim, or final result return as appropriate.

This also prevents accidental split wiring where one component starts a process and another reads results or signals cancellation from a different store/registry.

Tenant/user/agent/project scope still comes from `ExecutionContext.resource_scope`, and all persistence remains under the lower-level store contracts.

---

## 5. Live example

A non-Docker in-memory live example is available at:

```text
crates/ironclaw_host_runtime/examples/reborn_host_runtime.rs
```

Run it with:

```bash
cargo run -p ironclaw_host_runtime --example reborn_host_runtime
```

A non-Docker filesystem-backed live example is available at:

```text
crates/ironclaw_host_runtime/examples/reborn_host_runtime_filesystem.rs
```

Run it with:

```bash
cargo run -p ironclaw_host_runtime --example reborn_host_runtime_filesystem
```

The filesystem example uses `ProcessServices::filesystem(...)` and verifies that result metadata and JSON output are written under scoped artifact refs:

```text
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/process-results/{process_id}.json
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/process-outputs/{process_id}/output.json
```

Both examples use an in-process `ScriptBackend` with a manifest-declared `runner = "sandboxed_process"` script capability so they can demonstrate the full composition path without requiring Docker:

```text
HostRuntimeServices
  -> RuntimeDispatcher
  -> CapabilityHost::spawn_json(...)
  -> ProcessServices background manager
  -> ScriptRuntime + in-process backend
  -> ProcessHost::await_result/output(...)
```

---

## 6. Current non-goals

This slice does not implement:

- a full production `AppBuilder` replacement
- DB-backed host service factories
- channel adapters or turn service wiring
- thread/step stores
- message bus, durable subscription cursors, or event fanout
- runtime backend lifecycle management beyond holding shared handles
- policy configuration loading

Those should layer on this composition root rather than expanding dispatcher or capability-host responsibilities.
