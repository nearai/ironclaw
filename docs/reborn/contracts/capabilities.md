# IronClaw Reborn capabilities invocation contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_capabilities`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capability-access.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/dispatcher.md`

---

## 1. Purpose

`ironclaw_capabilities` is the caller-facing capability invocation service.

It keeps callers simple without making the runtime dispatcher own authorization:

```text
caller/channel/agent/conversation
  -> CapabilityHost::invoke_json(...)
      -> AuthorizationService / GrantAuthorizer
      -> optional RunStateStore / ApprovalRequestStore
      -> RuntimeDispatcher
          -> WASM / Script / MCP
```

This service is the middle communication layer between authorization and dispatch.

---

## 2. Responsibilities

`CapabilityHost` owns the high-level invocation workflow:

```text
1. receive ExecutionContext + capability id + input + estimate
2. validate ExecutionContext/resource_scope consistency before persistence or dispatch
3. if configured, mark invocation `Running` in `RunStateStore` under `context.resource_scope`
4. lookup CapabilityDescriptor in ExtensionRegistry
5. call CapabilityDispatchAuthorizer
6. if denied, mark `Failed` and return a typed invocation error before dispatch/resource reservation
7. if approval is required, save a tenant/user-scoped pending approval request, mark `BlockedApproval`, and return a typed approval-required error
8. if allowed, call CapabilityDispatcher with context.resource_scope
9. mark `Completed` or `Failed` after dispatch
10. return the normalized dispatch result
```

It does not implement grant matching itself; that belongs to `ironclaw_authorization`.
It does not select WASM/Script/MCP; that belongs to `ironclaw_dispatcher` behind the narrow `CapabilityDispatcher` interface.

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
```

The stores are optional for low-level tests, but host-facing invocation should configure them so approvals and failures are visible outside the call stack and can survive process restarts. The durable implementations write through tenant/user partitions under the `/engine` filesystem namespace, so production can provide a DB-backed filesystem implementation without coupling this crate to a specific database.

The capability host is responsible for preserving `ExecutionContext.resource_scope` across run-state, approval persistence, and dispatch. A caller cannot authorize under one tenant/user and persist or bill under another.

---

## 5. Relationship to dispatcher

`CapabilityHost` depends on the narrow `CapabilityDispatcher` trait. `RuntimeDispatcher` implements that trait and remains deliberately lower level:

```text
already-authorized dispatch request -> runtime lane -> normalized result
```

It has no dependency on `ironclaw_authorization`, no `ExecutionContext`, and no grant logic.

---

## 6. Current non-goals

This slice does not implement:

- invocation resume after approval resolution
- durable grant/lease storage, revocation, or expiration enforcement
- invocation count tracking
- obligation application beyond returning allowed/denied
- transcript/job history

Those belong to later capability-host/run-state/auth slices.
