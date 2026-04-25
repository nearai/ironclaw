# IronClaw Reborn capabilities invocation contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_capabilities`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capability-access.md`, `docs/reborn/contracts/dispatcher.md`

---

## 1. Purpose

`ironclaw_capabilities` is the caller-facing capability invocation service.

It keeps callers simple without making the runtime dispatcher own authorization:

```text
caller/channel/agent/conversation
  -> CapabilityHost::invoke_json(...)
      -> AuthorizationService / GrantAuthorizer
      -> RuntimeDispatcher
          -> WASM / Script / MCP
```

This service is the middle communication layer between authorization and dispatch.

---

## 2. Responsibilities

`CapabilityHost` owns the high-level invocation workflow:

```text
1. receive ExecutionContext + capability id + input + estimate
2. lookup CapabilityDescriptor in ExtensionRegistry
3. call CapabilityDispatchAuthorizer
4. if denied, return a typed invocation error before dispatch/resource reservation
5. if approval is required, return a typed approval-required error for future run-state handling
6. if allowed, call RuntimeDispatcher with context.resource_scope
7. return the normalized dispatch result
```

It does not implement grant matching itself; that belongs to `ironclaw_authorization`.
It does not select WASM/Script/MCP; that belongs to `ironclaw_dispatcher`.

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

## 4. Relationship to dispatcher

`RuntimeDispatcher` is now deliberately lower level:

```text
already-authorized dispatch request -> runtime lane -> normalized result
```

It has no dependency on `ironclaw_authorization`, no `ExecutionContext`, and no grant logic.

---

## 5. Current non-goals

This slice does not implement:

- approval request persistence
- run-state transitions such as `blocked_approval`
- grant storage, revocation, or expiration enforcement
- invocation count tracking
- obligation application beyond returning allowed/denied
- transcript/job history

Those belong to later capability-host/run-state/auth slices.
