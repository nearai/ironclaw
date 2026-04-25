# IronClaw Reborn host runtime composition contract

**Date:** 2026-04-25
**Status:** V1 composition slice
**Crate:** `crates/ironclaw_host_runtime`
**Depends on:** `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/capability-access.md`

---

## 1. Purpose

`ironclaw_host_runtime` is a composition-only crate for the current Reborn host/runtime vertical slice.

It wires already-owned service crates together so application setup code does not repeatedly hand-connect the same shared handles:

```text
ExtensionRegistry
RootFilesystem
ResourceGovernor
CapabilityDispatchAuthorizer
RunStateStore / ApprovalRequestStore / CapabilityLeaseStore
ProcessServices
WASM / Script / MCP runtime backends
EventSink
  -> HostRuntimeServices
      -> RuntimeDispatcher
      -> CapabilityHost
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
```

It may hold shared `Arc` handles to configured services and runtime backends.

It must not:

- implement grant matching or spawn policy
- execute runtime lanes directly
- own process state transitions or cancellation semantics
- own approval resolution or lease semantics
- expose process lifecycle APIs through `CapabilityHost`
- turn process subscriptions into a message bus
- weaken tenant/user scoped persistence boundaries

Ownership remains:

```text
authorization -> grant, lease, and spawn decisions
capabilities  -> caller-facing invoke/resume/spawn workflow
processes     -> lifecycle, result, output, cancellation, and process services
dispatcher    -> already-authorized runtime routing
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
.with_event_sink(event_sink);

let dispatcher = services.runtime_dispatcher_arc();
let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);
let process_host = services.process_host();
```

For tests or custom process executors, callers can also provide an arbitrary dispatcher and executor:

```rust
let capability_host = services.capability_host(&dispatcher, executor);
```

`capability_host_for_runtime_dispatcher(...)` derives a `DispatchProcessExecutor` from the same runtime dispatcher used for immediate dispatch. Spawned capability-backed process work therefore routes through the authorized dispatch interface after `CapabilityHost::spawn_json(...)` has authorized and recorded the process start.

---

## 4. Tenant and lifecycle invariants

The helper preserves the same service handles for `CapabilityHost` and `ProcessHost`:

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

This prevents accidental split wiring where one component starts a process and another reads results or signals cancellation from a different store/registry.

Tenant/user scope still comes from `ExecutionContext.resource_scope`, and all persistence remains under the lower-level store contracts.

---

## 5. Current non-goals

This slice does not implement:

- a full production `AppBuilder` replacement
- DB-backed host service factories
- channel adapters or turn service wiring
- thread/step stores
- message bus, durable subscription cursors, or event fanout
- runtime backend lifecycle management beyond holding shared handles
- policy configuration loading

Those should layer on this composition root rather than expanding dispatcher or capability-host responsibilities.
