# IronClaw Reborn capability access contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_authorization`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/kernel-dispatch.md`

---

## 1. Purpose

`ironclaw_authorization` evaluates authority-bearing host API contracts before runtime execution.

The first slice adds a grant-backed capability dispatch gate:

```text
ExecutionContext + CapabilityDescriptor + ResourceEstimate
  -> CapabilityDispatchAuthorizer::authorize_dispatch(...)
  -> Decision::Allow | Decision::Deny | Decision::RequireApproval
```

The authorizer does not execute capabilities, reserve resources, prompt users, inspect runtime internals, or discover extensions.

---

## 2. Default-deny rule

A registered capability is only a possibility. It is not authority.

V1 dispatch authorization requires a matching `CapabilityGrant` in `ExecutionContext.grants`:

```text
grant.capability == descriptor.id
AND grant.grantee matches the execution context principal
AND grant.constraints.allowed_effects covers descriptor.effects
```

If no matching grant exists, authorization returns:

```rust
Decision::Deny { reason: DenyReason::MissingGrant }
```

If a grant exists but does not cover the capability's declared effects, authorization returns:

```rust
Decision::Deny { reason: DenyReason::PolicyDenied }
```

If the `ExecutionContext` is internally inconsistent, authorization returns:

```rust
Decision::Deny { reason: DenyReason::InternalInvariantViolation }
```

---

## 3. Principal matching

The V1 `GrantAuthorizer` can match grants issued to:

- tenant
- user
- project
- mission
- thread
- extension

`Principal::System` is not matched as a grantee in this slice. System authority should remain explicit and narrow, not a wildcard grants bypass.

---

## 4. Kernel integration

`RuntimeDispatcher` can be configured with a capability access gate:

```rust
RuntimeDispatcher::new(&registry, &filesystem, &governor)
    .with_capability_authorizer(&authorizer, &execution_context)
```

When configured, dispatch performs authorization after descriptor/package consistency checks and before runtime selection:

```text
dispatch_requested event
lookup capability descriptor
lookup provider package
verify descriptor runtime matches package runtime
verify request.scope == execution_context.resource_scope
authorize dispatch
runtime_selected event
runtime executor call
```

Authorization denial happens before resource reservation and before runtime execution.

Denied authorization emits `dispatch_failed` with `error_kind = "AuthorizationDenied"`.

---

## 5. Current limits

This slice intentionally keeps authorization narrow:

- no approval prompt orchestration yet
- no grant persistence or revocation store
- no invocation count tracking for `max_invocations`
- no expiration enforcement yet
- no resource ceiling obligation enforcement yet
- no network/secret/mount policy injection into runtimes yet

Those should be added as follow-on slices once the pre-dispatch gate is stable.
