# IronClaw Reborn capability access contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_authorization`
**Depends on:** `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

`ironclaw_authorization` evaluates authority-bearing host API contracts before runtime execution.

The first slices add grant- and lease-backed capability dispatch/spawn gates:

```text
ExecutionContext + CapabilityDescriptor + ResourceEstimate
  -> CapabilityDispatchAuthorizer::authorize_dispatch(...) / authorize_spawn(...)
  -> Decision::Allow | Decision::Deny | Decision::RequireApproval
```

The authorizer does not execute capabilities, reserve resources, prompt users, inspect runtime internals, or discover extensions. Authorization and lease-store access are async so durable filesystem/DB-backed stores do not block async control-plane paths.

---

## 2. Default-deny rule

A registered capability is only a possibility. It is not authority.

V1 dispatch authorization requires a matching `CapabilityGrant` from `ExecutionContext.grants` or from an active tenant/user/agent-scoped `CapabilityLease`:

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

V1 spawn authorization targets the same declared capability but requires `EffectKind::SpawnProcess` in addition to the capability descriptor's declared effects. A dispatch-only grant must not authorize background or long-lived process creation.

---

## 3. Principal matching

The V1 `GrantAuthorizer` can match grants issued to:

- tenant
- user
- project
- mission
- thread
- extension

`Principal::System` is not matched as a grantee in this slice. System authority should remain explicit and narrow, not a wildcard grants bypass. Shipped loops and first-party packages do not become `System` merely because the project authored them; they still need host-policy-assigned trust ceilings plus explicit grants for privileged capability use.

---

## 4. Lease-backed authorization

Approved requests can issue `CapabilityLease` values:

```rust
pub struct CapabilityLease {
    pub scope: ResourceScope,
    pub grant: CapabilityGrant,
    pub invocation_fingerprint: Option<InvocationFingerprint>,
    pub status: CapabilityLeaseStatus,
}
```

`LeaseBackedAuthorizer` combines request-local grants with active non-fingerprinted leases visible to the current `ExecutionContext.resource_scope` and then applies the same grant matching rules. Fingerprinted approval leases are resume-only authority: they are not exposed as generic grants to `CapabilityHost::invoke_json`, because replay must first compare the approved `InvocationFingerprint` and claim the lease through `CapabilityHost::resume_json`.

Lease lookup is tenant/user/agent/invocation scoped; a lease issued under one tenant/user/agent/invocation must not authorize another tenant/user/agent/invocation, even when UUIDs collide. V1 approval leases are exact-invocation leases until reusable approval scopes are explicitly implemented.

V1 supports active, claimed, consumed, and revoked lease state. Revocation, claim, and consumption are tenant/user/agent/invocation scoped. Consumed, claimed, revoked, expired, exhausted, and fingerprinted approval leases are ignored by generic lease-backed authorization.

`CapabilityLeaseStore` is async:

```rust
#[async_trait]
pub trait CapabilityLeaseStore: Send + Sync {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError>;
    async fn get(&self, scope: &ResourceScope, lease_id: CapabilityGrantId) -> Option<CapabilityLease>;
    async fn revoke(&self, scope: &ResourceScope, lease_id: CapabilityGrantId) -> Result<CapabilityLease, CapabilityLeaseError>;
    async fn claim(...);
    async fn consume(...);
    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease>;
    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease>;
}
```

Lease storage implementations now include:

- `InMemoryCapabilityLeaseStore` for tests and ephemeral composition.
- `FilesystemCapabilityLeaseStore` for durable virtual-path persistence under `/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/capability-leases/{invocation_id}/{lease_id}.json`.

The filesystem store persists issue, claim, consume, and revoke transitions with awaited filesystem operations; it must not use nested `block_on` inside async approval/resume paths. Reads are fail-closed for authorization: unreadable or missing lease records do not become ambient grants.

See `docs/reborn/contracts/approvals.md` for how approval resolution issues leases and how resume claims/consumes them.

---

## 5. Capability host integration

`ironclaw_authorization` is consumed by the caller-facing capability invocation service, not by the dispatcher.

```text
CapabilityHost::invoke_json(...)
  -> GrantAuthorizer::authorize_dispatch(...)
  -> RuntimeDispatcher::dispatch_json(...)

CapabilityHost::spawn_json(...)
  -> GrantAuthorizer::authorize_spawn(...)
  -> ProcessManager::spawn(...)
```

Authorization denial happens before runtime dispatch, process creation, and resource reservation.

The dispatcher remains auth-unaware: it receives already-authorized `CapabilityDispatchRequest` values from `CapabilityHost` or another trusted host service.

---

## 6. Current limits

This slice intentionally keeps authorization narrow:

- no approval prompt UI/orchestration yet
- no durable grant persistence yet
- no transactional approval-record + lease write boundary yet
- no resource ceiling obligation enforcement yet
- no network/secret/mount policy injection into runtimes yet

Those should be added as follow-on slices once the pre-dispatch gate is stable.
