# IronClaw Reborn capability access contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_authorization`
**Depends on:** `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

`ironclaw_authorization` evaluates authority-bearing host API contracts before runtime execution.

The first slices add grant- and lease-backed capability dispatch gates:

```text
ExecutionContext + CapabilityDescriptor + ResourceEstimate
  -> CapabilityDispatchAuthorizer::authorize_dispatch(...)
  -> Decision::Allow | Decision::Deny | Decision::RequireApproval
```

The authorizer does not execute capabilities, reserve resources, prompt users, inspect runtime internals, or discover extensions.

---

## 2. Default-deny rule

A registered capability is only a possibility. It is not authority.

V1 dispatch authorization requires a matching `CapabilityGrant` from `ExecutionContext.grants` or from an active tenant/user-scoped `CapabilityLease`:

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

## 4. Lease-backed authorization

Approved requests can issue `CapabilityLease` values:

```rust
pub struct CapabilityLease {
    pub scope: ResourceScope,
    pub grant: CapabilityGrant,
    pub status: CapabilityLeaseStatus,
}
```

`LeaseBackedAuthorizer` combines request-local grants with active leases visible to the current `ExecutionContext.resource_scope` and then applies the same grant matching rules. Lease lookup is tenant/user scoped; a lease issued under one tenant/user must not authorize another tenant/user, even when UUIDs collide. V1 approval leases are also exact-invocation leases: they must not authorize a different invocation in the same tenant/user until reusable approval scopes are explicitly implemented.

V1 supports active and revoked lease state. Revocation is tenant/user scoped, and revoked leases are ignored by authorization.

See `docs/reborn/contracts/approvals.md` for how approval resolution issues leases.

---

## 5. Capability host integration

`ironclaw_authorization` is consumed by the caller-facing capability invocation service, not by the dispatcher.

```text
CapabilityHost::invoke_json(...)
  -> GrantAuthorizer::authorize_dispatch(...)
  -> RuntimeDispatcher::dispatch_json(...)
```

Authorization denial happens before runtime dispatch and before resource reservation.

The dispatcher remains auth-unaware: it receives already-authorized `CapabilityDispatchRequest` values from `CapabilityHost` or another trusted host service.

---

## 6. Current limits

This slice intentionally keeps authorization narrow:

- no approval prompt UI/orchestration yet
- no durable grant/lease persistence yet
- no invocation count tracking for `max_invocations`
- no expiration enforcement yet
- no resource ceiling obligation enforcement yet
- no network/secret/mount policy injection into runtimes yet

Those should be added as follow-on slices once the pre-dispatch gate is stable.
