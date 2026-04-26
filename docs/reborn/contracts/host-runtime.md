# IronClaw Reborn host runtime composition contract

**Date:** 2026-04-25
**Status:** V1 composition slice
**Crate:** `crates/ironclaw_host_runtime`
**Depends on:** `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/capability-access.md`

---

## 1. Purpose

`ironclaw_host_runtime` is a composition-only crate for the current Reborn host/runtime vertical slice.

Terminology note: older architecture docs use **kernel** for the small host-core composition layer. In the current implementation, that concrete composition crate is `ironclaw_host_runtime`; there is no active `ironclaw_kernel` crate in the Reborn stack. Treat “kernel” as the architecture concept and `ironclaw_host_runtime` as the concrete crate name unless a deliberate rename happens later.

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

It may hold shared `Arc` handles to configured services, runtime backends, observability sinks, an optional WASM host HTTP client, and an optional capability-obligation handler. It adapts concrete runtime crates into `ironclaw_dispatcher::RuntimeAdapter` implementations when building `RuntimeDispatcher`, and it provides a small `BuiltinObligationHandler` for metadata-only obligations plus WASM network-policy handoff.

It must not:

- implement grant matching or spawn policy
- execute runtime lanes directly outside adapter wrappers
- own process state transitions or cancellation semantics
- own approval resolution or lease semantics; `approval_resolver()` only wires `ironclaw_approvals::ApprovalResolver`
- own broad runtime/input/output obligation semantics; `with_obligation_handler(...)` passes a shared `CapabilityObligationHandler` through to `CapabilityHost`, and `BuiltinObligationHandler` is limited to audit-before metadata plus `ApplyNetworkPolicy` preflight/hand-off to WASM host HTTP imports
- expose process lifecycle APIs through `CapabilityHost`
- turn process subscriptions into a message bus
- weaken tenant/user scoped persistence boundaries

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

If configured, the shared `CapabilityObligationHandler` is passed into each `CapabilityHost` built by this helper. `HostRuntimeServices::with_builtin_obligation_handler()` installs the built-in handler with the shared `NetworkObligationPolicyStore`. It supports `AuditBefore` by emitting a redacted `AuditStage::Before` audit envelope through the configured `AuditSink`, and supports `ApplyNetworkPolicy` by validating policy metadata and storing the scoped policy in `NetworkObligationPolicyStore`; without a policy store, `ApplyNetworkPolicy` fails closed. The WASM runtime adapter consumes that scoped policy during dispatch and wraps the configured `WasmHostHttp` with `WasmPolicyHttpClient`, so actual WASM `host.http_request_utf8` calls are checked before the host HTTP client is called. If a network policy is present for Script or MCP lanes, those adapters fail closed with `NetworkDenied` until they have enforceable network plumbing. This handoff still does not perform DNS resolution, credential injection, or egress reservation by itself. `InjectSecretOnce`, `AuditAfter`, `RedactOutput`, `EnforceOutputLimit`, resource reservation, and scoped-mount obligations remain unsupported and fail closed until their required runtime/input/output plumbing exists. Unsupported or failed handler outcomes remain fail-closed inside `CapabilityHost` before dispatch, process start, or approval lease claim.

This also prevents accidental split wiring where one component starts a process and another reads results or signals cancellation from a different store/registry.

Tenant/user scope still comes from `ExecutionContext.resource_scope`, and all persistence remains under the lower-level store contracts.

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
/engine/tenants/{tenant_id}/users/{user_id}/process-results/{process_id}.json
/engine/tenants/{tenant_id}/users/{user_id}/process-outputs/{process_id}/output.json
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
