# IronClaw Reborn capability access contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_authorization`
**Depends on:** `docs/reborn/contracts/host-api.md`

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

## 4. Capability host integration

`ironclaw_authorization` is consumed by the caller-facing capability invocation service, not by the dispatcher.

```text
CapabilityHost::invoke_json(...)
  -> GrantAuthorizer::authorize_dispatch(...)
  -> RuntimeDispatcher::dispatch_json(...)
```

Authorization denial happens before runtime dispatch and before resource reservation.

The dispatcher remains auth-unaware: it receives already-authorized `CapabilityDispatchRequest` values from `CapabilityHost` or another trusted host service.

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
