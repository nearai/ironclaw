# IronClaw Reborn — Host API Invariants and Authorization Model

**Status:** Draft for implementation planning  
**Date:** 2026-04-24  
**Related docs:**

- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-architecture-faq-decisions.md`
- `docs/reborn/2026-04-24-self-contained-crate-roadmap.md`
- `docs/reborn/2026-04-24-existing-code-reuse-map.md`
- `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

Before implementing Reborn crate-by-crate, define the host invariants that every crate must preserve.

This document is intentionally about **invariants**, not implementation. The exact Rust APIs can evolve, but the security/resource/audit rules below should hold across all runtime lanes:

- WASM
- MCP
- script runner
- first-party extensions
- future runtime providers

Core model:

```text
ExecutionContext + Action + Grants + Approvals + Policy + ResourceState -> Decision
```

No costed, privileged, or externally visible side effect should happen without a host decision.

---

## 2. Authorization law

All side-effectful or quota-limited actions must pass through a host authorization boundary.

Conceptually:

```rust
fn authorize(
    ctx: &ExecutionContext,
    action: &Action,
    grants: &GrantStore,
    approvals: &ApprovalStore,
    policy: &PolicyStore,
    resources: &ResourceState,
) -> Decision;
```

Implementation can split this across filesystem, resources, auth, network, approval, and audit gates, but the invariant is the same:

```text
action executes only if the host can prove it is allowed for this context
```

Default posture:

```text
missing grant     -> deny
ambiguous scope   -> deny
invalid path      -> deny
unknown capability -> deny
unknown secret    -> deny
unreserved budget -> deny or require reservation
policy conflict   -> choose the most restrictive decision
```

---

## 3. Core host types

These are the kinds of types `crates/ironclaw_host_api` should define first.

### 3.1 Identity and scope newtypes

Use strong types instead of raw strings for all authority-bearing identifiers.

```rust
pub struct TenantId(pub String);
pub struct UserId(pub String);
pub struct ProjectId(pub String);
pub struct MissionId(pub String);
pub struct ThreadId(pub String);
pub struct InvocationId(pub String);
pub struct ProcessId(pub String);
pub struct ExtensionId(pub String);
pub struct CapabilityId(pub String);
pub struct CapabilityGrantId(pub String);
pub struct SecretHandle(pub String);
```

V1 may map `TenantId` to `UserId` in local/single-user mode, but the type exists from day one so hosted multi-tenancy is not retrofitted later.

### 3.2 Path types

Do not use one generic `Path(pub String)` for every layer. Split path concepts by authority.

```rust
pub struct HostPath(pub std::path::PathBuf); // never exposed to extensions
pub struct VirtualPath(pub String);          // canonical namespace, e.g. /projects/x/...
pub struct ScopedPath(pub String);           // extension-visible alias, e.g. /workspace/a.txt
pub struct MountAlias(pub String);           // /workspace, /project, /memory, /tmp
```

Invariant:

```text
extensions and runtimes never receive raw HostPath values
```

Host paths are internal implementation details of filesystem backends.

### 3.3 Runtime and trust

```rust
pub enum RuntimeKind {
    Wasm,
    Mcp,
    Script,
    FirstParty,
    System,
}

pub enum TrustClass {
    Sandbox,
    UserTrusted,
    FirstParty,
    System,
}
```

Trust is an authority ceiling, not an automatic permission grant.

### 3.4 Execution context

The original sketch used `ProcessContext`. Reborn should generalize this to `ExecutionContext`, because not every invocation is a process. WASM invocations, MCP calls, and internal host actions all need the same authority model.

```rust
pub struct ExecutionContext {
    pub invocation_id: InvocationId,
    pub process_id: Option<ProcessId>,
    pub parent_process_id: Option<ProcessId>,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,

    pub extension_id: ExtensionId,
    pub runtime: RuntimeKind,
    pub trust: TrustClass,

    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub resource_scope: ResourceScope,
}
```

Required invariant:

```text
all audit, budget, filesystem, secret, network, dispatch, and spawn decisions can be traced back to an ExecutionContext
```

---

## 4. Grants, not declarations, confer authority

Capability declarations describe what an extension can provide. They do not, by themselves, authorize a caller to use that capability.

```rust
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub provider: ExtensionId,
    pub effects: Vec<EffectKind>,
    pub runtime: RuntimeKind,
}

pub struct CapabilityGrant {
    pub id: CapabilityGrantId,
    pub capability: CapabilityId,
    pub grantee: Principal,
    pub issued_by: Principal,
    pub constraints: GrantConstraints,
}
```

Invariant:

```text
dispatch(X) succeeds only if the caller owns or was delegated an active grant for capability X
```

A manifest can say:

```text
extension github declares github.search_issues
```

But a context may call it only if a grant/lease says:

```text
this user/thread/extension may call github.search_issues under these constraints
```

---

## 5. Action model

The authorization gate should reason over explicit action classes.

```rust
pub enum Action {
    ReadFile {
        path: ScopedPath,
    },
    WriteFile {
        path: ScopedPath,
        bytes: Option<u64>,
    },
    DeleteFile {
        path: ScopedPath,
    },
    ListDir {
        path: ScopedPath,
    },

    Dispatch {
        capability: CapabilityId,
        estimated_resources: ResourceEstimate,
    },
    Spawn {
        extension_id: ExtensionId,
        requested_capabilities: CapabilitySet,
        requested_mounts: MountView,
        estimated_resources: ResourceEstimate,
    },

    UseSecret {
        handle: SecretHandle,
        mode: SecretUseMode,
    },
    Network {
        target: NetworkTarget,
        method: NetworkMethod,
        estimated_bytes: Option<u64>,
    },

    ReserveResources {
        estimate: ResourceEstimate,
    },

    Approve {
        request: ApprovalRequest,
    },

    EmitExternalEffect {
        effect: EffectKind,
    },
}
```

Notes:

- `UseSecret` uses a `SecretHandle`, not a raw name/string value.
- `NetworkTarget` should be parsed and validated, not a free-form domain string.
- `Spawn` must request its desired capabilities and mounts up front.
- `Dispatch` and `Spawn` carry resource estimates so budgeting cannot be bolted on later.

---

## 6. Decision model

`Allow` should be able to carry obligations. Authorization often permits an action only with required follow-up behavior.

```rust
pub enum Decision {
    Allow {
        obligations: Vec<Obligation>,
    },
    Deny {
        reason: DenyReason,
    },
    RequireApproval {
        request: ApprovalRequest,
    },
}
```

Example obligations:

```rust
pub enum Obligation {
    AuditBefore,
    AuditAfter,
    RedactOutput,
    ReserveResources(ResourceReservation),
    UseScopedMounts(MountView),
    InjectSecretOnce(SecretLease),
    ApplyNetworkPolicy(NetworkPolicy),
    EnforceOutputLimit(u64),
}
```

Invariant:

```text
an action is not complete until all obligations attached to its Allow decision have been satisfied
```

---

## 7. Formal invariants

### 7.1 Extension isolation

An extension can access only its own extension-local state unless explicitly delegated.

```text
extension A cannot write /system/extensions/B/**
extension A cannot read /system/extensions/B/state/** unless delegated
extension A cannot modify its own manifest/capability declarations at runtime except through extension update flow
```

Allowed by default:

```text
/system/extensions/A/config/**  as /extension/config/**
/system/extensions/A/state/**   as /extension/state/**
/system/extensions/A/cache/**   as /extension/cache/**
/tmp/**                         as invocation-local scratch
```

### 7.2 Path containment

All extension-visible paths must normalize inside one of the context's mounted roots.

Denied examples:

```text
/workspace/../../system/extensions/other
/project/../../../users/alice/memory
/Users/alice/project/file.txt
file:///etc/passwd
```

Invariant:

```text
normalize(scoped_path, mount_view) either returns a contained VirtualPath or fails closed
```

### 7.3 Capability soundness

A capability call succeeds only with an active grant.

```text
dispatch(X, params) succeeds only if caller owns X or was delegated X by an authorized principal
```

Capability declaration is not enough. Capability grant is required.

### 7.4 Delegation attenuation

A child invocation cannot acquire more authority than its parent or grant policy allows.

```text
child.capabilities ⊆ parent.capabilities ∩ grant_policy.capabilities
child.mounts       ⊆ parent.mounts       ∩ grant_policy.mounts
child.network      ⊆ parent.network      ∩ grant_policy.network
child.secrets      ⊆ parent.secrets      ∩ grant_policy.secrets
child.budget       ⊆ parent.remaining_budget / resource cascade
```

Escalation requires explicit approval or a new grant from an authorized principal.

### 7.5 Secret safety

Secrets are referenced by handles. Raw secret values are not normal data.

Default rules:

```text
UseSecret(handle, InjectIntoRequest) may be allowed if grant/policy permits
UseSecret(handle, ReadRaw) is denied or requires explicit high-risk approval
secret material never appears in audit logs, model-visible tool output, or extension config
```

Invariant:

```text
raw secret material crosses only a host-mediated one-shot injection boundary unless explicitly granted otherwise
```

### 7.6 Network mediation

All outbound network access is mediated by `ironclaw_network` or an equivalent sandbox-enforced policy.

Default rules:

```text
no raw sockets for WASM/script extensions
MCP remote HTTP goes through network policy
MCP stdio servers run with scoped environment and mediated credentials
private IP / localhost / metadata endpoints are denied unless explicitly allowed
```

### 7.7 Budget and resource safety

Costed or quota-limited work must reserve resources before execution and reconcile afterward.

```text
reserve(scope, estimate) -> execute -> reconcile(actual) / release()
```

Resource scope includes tenant/org from day one:

```text
tenant/org -> user -> project -> mission -> thread -> sub-thread/invocation
```

Invariant:

```text
no LLM call, WASM invocation, MCP call, script run, mission tick, heartbeat, routine, or job may spend/consume quota without resource authorization
```

### 7.8 Approval soundness

Approvals are scoped. Approval for one action does not imply approval for a broader action.

Example:

```text
approve write /workspace/README.md
```

Does not imply:

```text
approve write /workspace/src/main.rs
approve write /workspace/**
approve network api.github.com
```

Reusable approvals must state their scope explicitly:

```text
allow writes under /workspace/docs/** for this thread
allow github.search_issues for this project for 24h
```

Invariant:

```text
approval matching is exact or policy-defined; never substring/heuristic authority
```

### 7.9 Auditability

Every external side effect has durable audit records before and after execution.

External side effects include:

- filesystem write/delete
- secret use
- network request
- capability dispatch
- process/script spawn
- extension install/update/remove
- budget reservation/denial/override
- approval grant/cancel
- mission tick/fire

Invariant:

```text
if a side effect is externally visible or changes durable state, it has a correlation id and durable audit trail
```

### 7.10 Runtime lane containment

Runtime lanes cannot bypass host authorization.

```text
WASM host imports call host services
MCP adapters call host services
script runner mounts and environment are scoped by host services
first-party extensions use the same host API shape as third-party extensions
```

### 7.11 Kernel minimality

Kernel composes gates and services. It does not embed product-specific authorization rules.

Invariant:

```text
policy lives in grants, manifests, resource scopes, auth/network/filesystem services, and approval records — not hidden inside kernel product logic
```

---

## 8. Gate composition order

The implementation can compose multiple gates, but they should converge on one final decision.

Recommended conceptual order:

```text
1. context validation
2. path/capability/target normalization
3. grant check
4. policy check
5. approval check
6. resource reservation check
7. obligation construction
8. audit-before event
9. execution
10. audit-after event / reconcile / release
```

Most restrictive decision wins:

```text
Deny > RequireApproval > Allow
```

If any gate denies, execution stops.

---

## 9. Initial type checklist for `ironclaw_host_api`

The first host API contract should include at least:

```text
TenantId
UserId
ProjectId
MissionId
ThreadId
InvocationId
ProcessId
ExtensionId
CapabilityId
CapabilityGrantId
SecretHandle

RuntimeKind
TrustClass
Principal
ExecutionContext
CapabilityDescriptor
CapabilityGrant
CapabilitySet
GrantConstraints
MountView
HostPath
VirtualPath
ScopedPath
MountAlias
SecretUseMode
NetworkTarget
NetworkMethod
ResourceScope
ResourceEstimate
ResourceUsage
Action
Decision
DenyReason
ApprovalRequest
Obligation
AuditEventKind
CorrelationId
```

Not every type needs full behavior in PR 1, but the names and authority boundaries should be stable.

---

## 10. Required invariant tests

Each crate should eventually contribute tests to this matrix.

### Host API / policy tests

- unknown capability denies dispatch
- declared-but-ungranted capability denies dispatch
- active grant allows dispatch under constraints
- expired/revoked grant denies dispatch
- child grant cannot exceed parent grant
- approval for one path does not match a different path

### Filesystem tests

- extension A cannot read/write extension B state
- scoped path traversal is denied
- raw host paths are rejected at host API boundary
- symlink escape is denied by filesystem backend
- `/tmp` is invocation-local and not shared across unrelated invocations

### Resources tests

- reservation denied when tenant/user/project scope is exhausted
- concurrent reservations cannot oversubscribe one ledger
- reconciliation releases over-reservation
- release records no spend
- zero-USD local model still respects token/runtime quotas

### Auth/network tests

- secret handle injection succeeds only for permitted dispatch
- raw secret read denied by default
- private IP network target denied by default
- network call without network grant denied

### Audit tests

- write/delete/network/secret/dispatch/spawn actions produce before/after audit events
- budget override creates an auditable approval/override event
- denied action records a denial event without performing the effect

### Runtime lane tests

- WASM cannot access filesystem except host imports
- WASM cannot perform network except mediated host import
- MCP stdio server receives scoped environment only
- script runner cannot escape mounts or inherit raw host environment

---

## 11. Implementation warning

Do not start by coding a large `authorize()` function in the kernel.

Start by defining the host API contracts and invariant tests. Then each service implements its own piece:

```text
ironclaw_filesystem -> path containment and mount authority
ironclaw_resources  -> budget/resource reservation authority
ironclaw_extensions -> manifest/grant/capability declarations
ironclaw_auth       -> secret handle authority
ironclaw_network    -> egress authority
runtime lanes       -> no-bypass execution boundaries
ironclaw_kernel     -> composition only
```

The goal is not one monolithic policy engine. The goal is one coherent authorization law enforced by narrow services.
