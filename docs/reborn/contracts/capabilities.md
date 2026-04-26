# IronClaw Reborn capabilities invocation contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_capabilities`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capability-access.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/processes.md`, `docs/reborn/contracts/dispatcher.md`

---

## 1. Purpose

`ironclaw_capabilities` is the caller-facing capability invocation service.

It keeps callers simple without making the runtime dispatcher own authorization:

```text
caller/channel/agent/conversation
  -> CapabilityHost::invoke_json(...) / resume_json(...) / spawn_json(...)
      -> AuthorizationService / GrantAuthorizer / LeaseBackedAuthorizer
      -> optional RunStateStore / ApprovalRequestStore / CapabilityLeaseStore / CapabilityObligationHandler / ProcessManager
      -> CapabilityDispatcher port or ProcessManager
          -> WASM / Script / MCP or tracked/background ProcessRecord
```

This service is the middle communication layer between authorization, dispatch, and process lifecycle start workflows.

---

## 2. Responsibilities

`CapabilityHost` owns the high-level invocation workflow:

```text
1. receive ExecutionContext + capability id + input + estimate
2. validate ExecutionContext/resource_scope consistency before persistence or dispatch
3. compute an `InvocationFingerprint` over scope + capability + estimate + JSON input without storing raw input in the approval record
4. if configured, mark invocation `Running` in `RunStateStore` under `context.resource_scope`
5. lookup CapabilityDescriptor in ExtensionRegistry
6. call CapabilityDispatchAuthorizer
7. if denied, mark `Failed` and return a typed invocation error before dispatch/resource reservation
8. if approval is required, require coherent run-state/approval-store wiring, attach/validate the invocation fingerprint, save a tenant/user-scoped pending approval request, mark `BlockedApproval`, and return a typed approval-required error
9. if allowed with empty obligations, call CapabilityDispatcher with context.resource_scope
10. if allowed with non-empty obligations and a configured handler satisfies them, continue to dispatch
11. if obligations are unsupported, unconfigured, or handler-failed, mark `Failed` and return before dispatch
12. mark `Completed` or `Failed` after dispatch
13. return the normalized dispatch result
```

`CapabilityHost::resume_json` owns the approved-resume workflow:

```text
1. receive ExecutionContext + approval request id + capability id + input + estimate
2. validate ExecutionContext/resource_scope consistency
3. load the blocked run from RunStateStore under the same scope
4. load the approval record and require status Approved
5. recompute InvocationFingerprint and compare it to the approved request fingerprint
6. find an unexpired active lease for the same tenant/user/invocation, capability, and fingerprint
7. call CapabilityDispatchAuthorizer with the matching lease grant as request-local authority
8. if allowed with non-empty obligations and a configured handler satisfies them, continue without claiming the lease yet
9. if obligations are unsupported, unconfigured, or handler-failed, mark `Failed` and return before claiming the lease or dispatching
10. claim the matching lease before runtime dispatch so concurrent resumes cannot dispatch with the same one-shot lease
11. call CapabilityDispatcher
12. consume the claimed lease after successful dispatch
13. mark Completed or Failed
```

`CapabilityHost::spawn_json` owns the capability-backed process start workflow:

```text
1. receive ExecutionContext + capability id + input + estimate
2. validate ExecutionContext/resource_scope consistency
3. if configured, mark invocation `Running` in `RunStateStore`
4. lookup CapabilityDescriptor in ExtensionRegistry
5. call CapabilityDispatchAuthorizer::authorize_spawn, requiring `SpawnProcess` plus descriptor effects
6. if allowed with empty obligations, ask ProcessManager to create a tenant/user-scoped ProcessRecord and optionally launch background execution
7. if allowed with non-empty obligations and a configured handler satisfies them, continue to process creation
8. if obligations are unsupported, unconfigured, or handler-failed, mark `Failed` and return before process creation
9. mark the start invocation Completed or Failed
10. return the ProcessRecord with ProcessId, scope, extension_id, capability_id, runtime, grants, mounts, and status
```

Spawn is capability-targeted. It does not start raw host processes or extension-level workers without a declared capability identity.

It does not implement grant matching itself; that belongs to `ironclaw_authorization`.
It does not select WASM/Script/MCP for dispatch; that belongs to a concrete implementation such as `ironclaw_dispatcher` behind the neutral `ironclaw_host_api::CapabilityDispatcher` port. The `DispatchProcessExecutor` adapter can run spawned process input through that same dispatch interface from a background process manager and verifies the returned provider/runtime still match the process record.
It does not own process lifecycle or process-result mechanics after start; those belong to `ironclaw_processes` behind `ProcessManager`/`ProcessStore`/`ProcessResultStore`. Applications can use `ProcessServices` to compose those process pieces consistently before handing a `ProcessManager` to `CapabilityHost`.

Dispatch failures are reported through sanitized host-safe dispatch error kinds; `CapabilityInvocationError` does not expose boxed runtime/backend error details.

Authorization obligations are satisfied through a host-provided `CapabilityObligationHandler` seam. `CapabilityHost` calls the handler after `Decision::Allow { obligations }` and before dispatch, process start, or approval lease claim. If no handler is configured, if the handler reports unsupported obligations, or if the handler fails, the invocation fails closed before downstream side effects. Handler failures use the stable `CapabilityObligationFailureKind` categories and must remain sanitized metadata, not raw input, host paths, secret material, runtime output, or detailed backend errors. This prevents authorizers from attaching requirements such as audit, output limits, network policy, secret injection, or resource reservations that callers silently ignore.

---

## 3. Caller API

```rust
let result = capability_host
    .invoke_json(CapabilityInvocationRequest {
        context,
        capability_id,
        estimate,
        input,
    })
    .await?;
```

The caller provides the `ExecutionContext`; it does not manually evaluate grants or call the dispatcher.

For spawn, callers use the same host-facing pattern. Applications can either provide an explicit `ProcessManager`, let the host derive one from `ProcessServices` and a background executor, or use `HostRuntimeServices` from `ironclaw_host_runtime` to build the dispatcher/capability-host/process-host handles together:

```rust
let services = ProcessServices::in_memory();
let executor = Arc::new(DispatchProcessExecutor::new(dispatcher.clone()));
let capability_host = CapabilityHost::new(&registry, dispatcher.as_ref(), &authorizer)
    .with_process_services(&services, executor);

let result = capability_host
    .spawn_json(CapabilitySpawnRequest {
        context,
        capability_id,
        estimate,
        input,
    })
    .await?;

let output = services.host().output(&result.process.scope, result.process.process_id).await?;
```

The host service builds the lower-level dispatch request using:

```rust
scope: request.context.resource_scope
```

so callers cannot accidentally provide an authorization context for one scope and dispatch billing/work under another scope.

---

## 4. Relationship to run-state

`CapabilityHost` is the first owner of invocation workflow state:

```rust
CapabilityHost::new(&registry, &dispatcher, &authorizer)
    .with_run_state(&run_state)
    .with_approval_requests(&approval_requests)
    .with_capability_leases(&leases)
    .with_obligation_handler(&obligations)
    .with_process_manager(&processes)
```

The stores are optional for low-level tests, but host-facing invocation should configure them so approvals and failures are visible outside the call stack and can survive process restarts. The durable implementations write through tenant/user partitions under the `/engine` filesystem namespace, so production can provide a DB-backed filesystem implementation without coupling this crate to a specific database.

The capability host is responsible for preserving `ExecutionContext.resource_scope` across run-state, approval persistence, and dispatch. A caller cannot authorize under one tenant/user and persist or bill under another.

For approval-required dispatches, `CapabilityHost` also binds the approval to the exact invocation request by attaching an `InvocationFingerprint`. If an authorizer supplies a conflicting fingerprint, the host fails the run with `InvocationFingerprintMismatch` and persists no approval request.

If only one of `RunStateStore` or `ApprovalRequestStore` is configured and authorization requires approval, `CapabilityHost` fails closed instead of creating a non-resumable blocked run or orphan approval request. Host-facing approval paths should configure both stores.

For approved resume, `CapabilityHost` compares the replayed request fingerprint to the approved fingerprint before dispatch, then satisfies any authorization obligations before claiming the matching lease. Denied/expired/non-approved approvals, missing leases, failed lease claims, fingerprint mismatches, and unsupported obligation handling fail before runtime dispatch. The lease is claimed immediately before runtime dispatch and consumed after successful dispatch.

For spawn, `CapabilityHost` preserves `ExecutionContext.resource_scope` and creates a process record through `ProcessManager`. It does not call `dispatch_json` directly. If a `ResourceManagedProcessStore` is configured behind that manager, process resource reservations are owned and cleaned up by `ironclaw_processes`, not by the capability host or dispatcher. If a runtime-backed process manager is configured, the lower-level `DispatchProcessExecutor` adapter can route the background work through `CapabilityDispatcher` after the authorized process record is created. When a process already owns a reservation, that adapter suppresses a duplicate runtime reservation for the same process estimate and validates the dispatch result provider/runtime against the process record. `CapabilityHost::with_process_services(...)` is convenience wiring for that background-manager path; it derives a process manager from shared `ProcessServices` so later `ProcessHost` status/kill/result/output operations see the same process store, result store, and cancellation registry. If an `EventingProcessStore` is used behind that manager, process lifecycle events are emitted by `ironclaw_processes`, not by the capability host or dispatcher. The process record carries the target capability identity and runtime so later lifecycle operations remain capability-backed. Host-facing lifecycle operations after spawn belong to `ironclaw_processes::ProcessHost`, not to `CapabilityHost`.

---

## 5. Relationship to dispatcher

`CapabilityHost` depends on the narrow `ironclaw_host_api::CapabilityDispatcher` trait. `RuntimeDispatcher` implements that trait and remains deliberately lower level:

```text
already-authorized dispatch request -> runtime lane -> normalized result
```

It has no dependency on `ironclaw_authorization`, no `ExecutionContext`, and no grant logic.

---

## 6. Current non-goals

This slice does not implement:

- durable grant/lease storage, revocation, or expiration persistence
- approval/resume of `Action::SpawnCapability`
- process lifecycle/cancellation/result APIs inside `CapabilityHost`; `status`/`kill`/`await_process`/`subscribe`/`result`/`await_result` and cooperative cancellation live in `ironclaw_processes`
- process output/result APIs inside `CapabilityHost`; result lookup and output resolution live in `ironclaw_processes`
- generalized streaming/binary process output references beyond the current filesystem JSON output path
- streaming output APIs
- built-in obligation semantics for secrets, network I/O, audit emission, output redaction, or output limiting; this slice only exposes the fail-closed handler seam
- transcript/job history

Those belong to later capability-host/run-state/auth slices.
