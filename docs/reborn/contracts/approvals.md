# IronClaw Reborn approval resolution contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_approvals`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capability-access.md`, `docs/reborn/contracts/run-state.md`

---

## 1. Purpose

`ironclaw_approvals` resolves durable approval requests into bounded authorization leases.

It is a host control-plane service. It does not prompt users, render UI, execute capabilities, reserve resources, or route runtime work.

The intended flow is:

```text
CapabilityHost
  -> Authorization returns RequireApproval
  -> ApprovalRequestStore saves Pending request under tenant/user scope
  -> RunStateStore marks invocation BlockedApproval

ApprovalResolver
  -> reads Pending ApprovalRecord under the same tenant/user scope
  -> approve: marks Approved and issues a scoped CapabilityLease
  -> deny: marks Denied and issues no lease

LeaseBackedAuthorizer
  -> combines ExecutionContext.grants with active scoped leases
  -> returns Allow/Deny before CapabilityHost dispatches runtime work
```

---

## 2. Approval request status transitions

Approval records live in `ironclaw_run_state` because they explain why an invocation is `BlockedApproval`.

The V1 status model is:

```rust
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}
```

`ApprovalRequestStore` supports scoped resolution methods:

```rust
async fn approve(scope, request_id) -> Result<ApprovalRecord, RunStateError>;
async fn deny(scope, request_id) -> Result<ApprovalRecord, RunStateError>;
```

All operations are tenant/user scoped. Resolving a request with the wrong tenant/user returns an unknown request error and must not reveal whether another tenant/user has a matching UUID.

---

## 3. Capability leases

Approved dispatch requests issue `CapabilityLease` values in `ironclaw_authorization`:

```rust
pub struct CapabilityLease {
    pub scope: ResourceScope,
    pub grant: CapabilityGrant,
    pub status: CapabilityLeaseStatus,
}
```

A lease wraps a normal `CapabilityGrant` so existing grant constraints remain the authority shape:

```text
capability
principal/grantee
allowed effects
mount/network/secret/resource constraints
expiry
max invocations
```

The lease adds host-managed lifecycle state:

```rust
pub enum CapabilityLeaseStatus {
    Active,
    Revoked,
}
```

V1 includes an in-memory lease store with tenant/user scoped lookup and revocation. Lease lookup and revocation are not global by ID; the authorizer asks for active leases visible to the current `ExecutionContext.resource_scope`. This slice treats issued approval leases as one-off invocation leases: a lease only authorizes a context with the same invocation ID as the approved request. Broader reusable approval scopes are a later policy slice.

---

## 4. Approval resolver

`ApprovalResolver` only resolves `Pending` records. Attempts to approve or deny an already-approved, denied, or expired record fail without changing that record.

`ApprovalResolver` turns a pending dispatch approval into a lease:

```rust
let lease = resolver
    .approve_dispatch(
        &scope,
        approval_request_id,
        LeaseApproval {
            issued_by,
            allowed_effects,
            expires_at,
            max_invocations,
        },
    )
    .await?;
```

For dispatch approvals, the lease grant uses:

```text
grant.capability = capability from Action::Dispatch
grant.grantee    = ApprovalRequest.requested_by
grant.issued_by  = LeaseApproval.issued_by
grant.constraints.allowed_effects = LeaseApproval.allowed_effects
grant.constraints.expires_at = LeaseApproval.expires_at
grant.constraints.max_invocations = LeaseApproval.max_invocations
```

Denying a request only transitions the approval record:

```rust
resolver.deny(&scope, approval_request_id).await?;
```

No lease is issued for denied requests.

---

## 5. Authorization integration

`LeaseBackedAuthorizer` evaluates both request-local grants and active leases:

```text
ExecutionContext.grants + CapabilityLeaseStore.active_grants_for_context(context)
  -> normal grant matching rules
  -> Decision::Allow | Decision::Deny
```

This preserves the dispatcher boundary:

```text
caller -> CapabilityHost -> authorizer -> CapabilityDispatcher -> RuntimeDispatcher -> runtime
```

The dispatcher remains auth-blind and state-blind. It never resolves approvals or inspects leases.

---

## 6. Current limits

This slice intentionally keeps approval resolution narrow:

- no UI/user prompt implementation
- no invocation resume API in `CapabilityHost` yet
- no durable lease store yet
- no lease expiration enforcement yet
- no invocation-count decrementing for `max_invocations` yet
- no atomic transaction across approval status update and lease issuance yet
- no approval resolution audit event yet
- no lease revocation persistence beyond the in-memory store
- no approval support for non-dispatch actions yet
- no reusable approval-scope expansion yet; V1 leases are exact-invocation only

Before user-facing approval resume ships, the host should revisit atomic persistence for:

```text
approval record update + lease/grant write + run-state transition
```
