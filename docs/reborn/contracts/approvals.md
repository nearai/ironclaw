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
  -> ApprovalRequestStore saves Pending request under tenant/user/agent scope
  -> RunStateStore marks invocation BlockedApproval

ApprovalResolver
  -> reads Pending ApprovalRecord under the same tenant/user/agent scope
  -> approve: durably issues a scoped CapabilityLease carrying the invocation fingerprint, then marks Approved
  -> deny: marks Denied and issues no lease
  -> optionally emits metadata-only AuditEnvelope::approval_resolved records

LeaseBackedAuthorizer
  -> combines ExecutionContext.grants with active non-fingerprinted scoped leases
  -> returns Allow/Deny before CapabilityHost dispatches runtime work

CapabilityHost::resume_json
  -> reloads the approved record and matching fingerprinted lease
  -> compares the replayed invocation fingerprint
  -> claims the lease before runtime dispatch
  -> dispatches and consumes the claimed lease on success
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

All operations are tenant/user/agent scoped. Resolving a request with the wrong tenant/user/agent returns an unknown request error and must not reveal whether another tenant/user/agent has a matching UUID.

---

## 3. Invocation fingerprints

Approval records may carry an `InvocationFingerprint`:

```rust
pub struct InvocationFingerprint(String);
```

For dispatch approvals, `CapabilityHost` computes the fingerprint from:

```text
version
kind = dispatch
ResourceScope, including invocation_id
CapabilityId
ResourceEstimate
canonical JSON input with object keys sorted recursively
```

The stored value is a `sha256:` digest, not raw JSON input. The resume path compares this digest before dispatch so an approval for one input cannot be replayed with a different input.

If an authorizer returns `Decision::RequireApproval` with no fingerprint, `CapabilityHost` attaches the computed one. If it returns a different fingerprint, `CapabilityHost` fails closed before saving the approval request.

---

## 4. Capability leases

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
    Claimed,
    Consumed,
    Revoked,
}
```

V1 includes in-memory and filesystem-backed lease stores with exact tenant/user/agent/invocation scoped lookup, claim, consumption, and revocation. Filesystem leases persist under `/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/capability-leases/{invocation_id}/{lease_id}.json`. Lease lookup, claim, consumption, and revocation are not global by ID; the authorizer asks for unexpired active leases visible to the current `ExecutionContext.resource_scope`. This slice treats issued approval leases as one-off invocation leases: a lease only authorizes a context with the same invocation ID as the approved request. Broader reusable approval scopes are a later policy slice.

Leases preserve the approval request fingerprint so resume can validate that the replayed invocation request matches what was approved. Fingerprinted approval leases are not converted into generic grants for plain `invoke_json`; they can only be used by `resume_json`, which compares the fingerprint and claims the exact lease before dispatch.

Claiming enforces that the lease is active, unexpired, not exhausted, and fingerprint-equal to the replayed request. A claimed lease is hidden from generic authorization so a second concurrent resume cannot also dispatch with the same one-shot approval lease.

Lease consumption enforces `GrantConstraints.max_invocations`:

```text
Some(n > 1) -> decrement and remain Active
Some(1)     -> decrement to Some(0) and mark Consumed
Some(0)     -> reject as exhausted
None        -> no invocation-count decrement
```

Expiration is enforced during authorization, claim, and consumption using `GrantConstraints.expires_at`.

---

## 5. Approval resolver

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
lease.invocation_fingerprint = ApprovalRequest.invocation_fingerprint
```

Denying a request only transitions the approval record and records the resolver actor:

```rust
resolver
    .deny(
        &scope,
        approval_request_id,
        DenyApproval {
            denied_by: Principal::User(scope.user_id.clone()),
        },
    )
    .await?;
```

No lease is issued for denied requests.

Approval resolution is ordered fail-closed around lease persistence:

1. Re-read the approval request and require `Pending`.
2. Build and persist the exact fingerprinted lease.
3. Only after lease persistence succeeds, mark the approval request `Approved`.
4. If the approval status write fails after lease persistence, attempt to revoke the issued lease before returning the run-state error.

This prevents an approval record from becoming `Approved` without durable resume authority, and with lease stores that can revoke the issued record it prevents an approval-write failure from leaving an active orphan lease. The resolver still spans separate stores, so this is a fail-closed coordination rule rather than a single database ACID transaction.

Approval resolution can also emit best-effort audit records when configured with an `AuditSink`:

```rust
let resolver = ApprovalResolver::new(&approvals, &leases).with_audit_sink(&audit);
```

Successful approve and deny transitions emit `AuditEnvelope::approval_resolved(...)` records with `AuditStage::ApprovalResolved`, the original approval correlation ID, the approval request ID, a summarized action, and `DecisionSummary { kind: "approved" | "denied", actor: Some(resolver principal), ... }`.

The audit records do not include approval reasons, replay input, invocation fingerprints, lease IDs, lease contents, raw host paths, or secret values. The same redaction contract applies when records are persisted through `JsonlAuditSink` at a tenant/user/agent scoped virtual path from `scoped_audit_log_path(&scope, "approval-audit.jsonl")`. Audit sink failures are ignored and must not change approval resolution outcomes.

---

## 6. Authorization integration

`LeaseBackedAuthorizer` evaluates request-local grants and active non-fingerprinted leases:

```text
ExecutionContext.grants + CapabilityLeaseStore.active_grants_for_context(context)
  -> normal grant matching rules
  -> Decision::Allow | Decision::Deny
```

Fingerprinted approval leases are deliberately excluded from `active_grants_for_context`. During resume, `CapabilityHost` first validates the approved fingerprint and claims the lease, then passes the claimed lease grant as request-local authority for the replayed dispatch. This keeps approval leases from becoming ambient same-invocation grants for plain `invoke_json`.

This preserves the dispatcher boundary:

```text
caller -> CapabilityHost -> authorizer -> CapabilityDispatcher -> RuntimeDispatcher -> runtime
```

The dispatcher remains auth-blind and state-blind. It never resolves approvals or inspects leases.

---

## 7. Current limits

This slice intentionally keeps approval resolution narrow:

- no UI/user prompt implementation
- no single-store ACID transaction across approval status update and lease issuance yet; resolver ordering and rollback provide fail-closed semantics across separate async stores
- no approval support for non-dispatch actions yet
- no `Action::SpawnCapability`/long-running task approval workflow yet; spawn start authorization exists, but approval/resume for spawn is a later slice
- no reusable approval-scope expansion yet; V1 leases are exact-invocation only

Before a durable/user-facing approval resume UI ships, the host should revisit whether approval records, lease writes, and run-state transitions should share one transactional persistence boundary.


---

## Contract freeze addendum — V1 lease scope (2026-04-25)

V1 approvals resolve to exact-invocation leases only.

A valid approval lease is bound to:

```text
tenant_id
user_id
project_id, if present
agent_id, if present
invocation_id
capability_id
invocation fingerprint
expiry/status
```

It authorizes one replay of the exact blocked invocation. It does not grant reusable scoped permission for future invocations. Scoped reusable approvals may be designed in V2 but must not be implied by V1 approval records.
