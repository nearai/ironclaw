# Reborn Contract — `ironclaw_host_api`

**Status:** Draft v0 contract
**Date:** 2026-04-24
**Target crate:** `crates/ironclaw_host_api`
**Source architecture docs:**

- `docs/reborn/2026-04-24-host-api-invariants-and-authorization.md`
- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-self-contained-crate-roadmap.md`
- `docs/reborn/2026-04-24-existing-code-reuse-map.md`

---

## 1. Purpose

`ironclaw_host_api` defines the shared vocabulary for Reborn authority boundaries.

It is not a runtime, policy engine, filesystem, budget ledger, or extension manager. It is the contract crate that gives all those crates the same names for:

- identities and scopes
- execution context
- paths and mount views
- capability descriptors and grants
- actions and decisions
- approvals and obligations
- resource estimates/usages
- audit/event envelopes

The first implementation PR should create this crate before implementing `ironclaw_filesystem`, `ironclaw_resources`, `ironclaw_extensions`, `ironclaw_wasm`, or `ironclaw_dispatcher`.

---

## 2. Dependency rules

### May depend on

- `serde`
- `serde_json`
- `uuid`
- `chrono` or `time`
- `rust_decimal`
- `thiserror`
- optionally `schemars` if JSON schema generation is useful

### Must not depend on

- `ironclaw_dispatcher`
- `ironclaw_filesystem`
- `ironclaw_resources`
- `ironclaw_extensions`
- `ironclaw_wasm`
- `ironclaw_mcp`
- `ironclaw_scripts`
- `ironclaw_auth`
- `ironclaw_network`
- current `src/tools/*`
- current `src/agent/*`

Hard invariant:

```text
ironclaw_host_api -> no system-service or runtime crates
```

---

## 3. Proposed module layout

```text
crates/ironclaw_host_api/src/
  lib.rs
  ids.rs
  path.rs
  mount.rs
  scope.rs
  runtime.rs
  capability.rs
  resource.rs
  approval.rs
  action.rs
  decision.rs
  audit.rs
  error.rs
```

Keep modules small. If a module starts needing runtime behavior, that behavior belongs in a service crate.

---

## 4. Serialization conventions

All host API types that cross crate/process boundaries should implement:

```rust
Debug
Clone
PartialEq
Eq where practical
Hash where practical
Serialize
Deserialize
```

Enums exposed over wire formats should use:

```rust
#[serde(rename_all = "snake_case")]
```

IDs should serialize as strings.

Durations should serialize as milliseconds or seconds with explicit field names, not ambiguous raw integers.

Money should use decimal strings or `rust_decimal::Decimal`, never `f64`, for ledger-facing values.

Time should use an explicit wrapper/type alias consistently:

```rust
pub type Timestamp = chrono::DateTime<chrono::Utc>;
```

If the implementation prefers `time::OffsetDateTime`, choose it once in PR 1 and use it everywhere in `ironclaw_host_api`.

---

## 5. ID and name contracts

### 5.1 System-generated IDs

Use UUID-backed IDs for runtime-created records:

```rust
pub struct InvocationId(pub Uuid);
pub struct ProcessId(pub Uuid);
pub struct CapabilityGrantId(pub Uuid);
pub struct ResourceReservationId(pub Uuid);
pub struct ApprovalRequestId(pub Uuid);
pub struct AuditEventId(pub Uuid);
pub struct CorrelationId(pub Uuid);
```

Rules:

- generated with `Uuid::new_v4()` unless a deterministic test constructor is explicitly marked test-only
- display as canonical lowercase UUID
- parse rejects non-UUID input

### 5.2 Scope IDs

Scope IDs may map to existing database or external identity IDs, so they are opaque strings with validation:

```rust
pub struct TenantId(pub String);
pub struct UserId(pub String);
pub struct ProjectId(pub String);
pub struct AgentId(pub String);
pub struct MissionId(pub String);
pub struct ThreadId(pub String);
```

Validation:

```text
1..=256 bytes
no NUL
no ASCII control characters
no slash or backslash
no raw dot-segment value: "." or ".."
```

V1 may use UUID strings for all of these, but the contract should not require UUID for user IDs if hosted identity providers use opaque stable IDs.

### 5.3 Stable package/capability names

Extension and capability names are authority-bearing and path-adjacent. They require stricter validation.

```rust
pub struct ExtensionId(pub String);
pub struct CapabilityId(pub String);
pub struct SecretHandle(pub String);
```

`ExtensionId` validation:

```text
1..=128 bytes
lowercase ASCII letters, digits, `_`, `-`, `.`
starts with lowercase ASCII letter or digit
no `/`, `\`, NUL, whitespace, or control characters
no `..` segment
```

`CapabilityId` validation:

```text
<extension_id>.<capability_name>
```

Where `capability_name` follows the same character rules as `ExtensionId` but may contain additional `.` segments.

Examples:

```text
github.search_issues
portfolio.lookup
script.run
```

Rejected:

```text
../github.search
github/search
github..search
GitHub.search
```

---

## 6. Runtime and trust contracts

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Wasm,
    Mcp,
    Script,
    FirstParty,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    Sandbox,
    UserTrusted,
    FirstParty,
    System,
}
```

Rules:

- `RuntimeKind::FirstParty` and `RuntimeKind::System` are concrete host-lane markers for host-policy-selected services in the broader `Host | WASM | Script Runner` model.
- `RuntimeKind::Mcp` is a capability adapter lane; local stdio MCP servers may still be process/sandbox-backed internally.
- `RuntimeKind::Script` is the native CLI/script lane. Docker/container is the V1 backend selected by policy, not a distinct public host API runtime kind.
- `TrustClass` is an authority ceiling, not a permission grant and not a kernel bypass.
- Shipped first-party code and bundled reference loops still need explicit grants, scoped mounts, resource reservations, leases, and obligation handling for privileged effects.
- User-installed packages cannot self-declare `TrustClass::FirstParty` or `TrustClass::System`; those ceilings are assigned only by host policy, signed/bundled package metadata, or admin configuration.
- Extension or loop upgrades retain grants only when package identity, signer/source policy, trust class, and requested authority remain valid; expanded authority requires renewed approval or admin policy.
- `System` is reserved for host-owned services and tests with explicit fixtures, and is not a matchable grantee for ordinary extension manifests.

---

## 7. Path contracts

Do not define one generic `Path(pub String)` for all layers.

### 7.1 Path types

```rust
pub struct HostPath(pub PathBuf);
pub struct VirtualPath(pub String);
pub struct ScopedPath(pub String);
pub struct MountAlias(pub String);
```

Meanings:

| Type | Meaning | Exposed to extensions? |
|---|---|---|
| `HostPath` | physical local path/backend implementation path | no |
| `VirtualPath` | canonical Reborn namespace path like `/projects/p1/threads/t1` | only through trusted host APIs |
| `ScopedPath` | extension-visible path like `/workspace/README.md` | yes |
| `MountAlias` | alias root such as `/workspace`, `/project`, `/memory`, `/tmp` | yes |

### 7.2 Virtual roots

V1 canonical virtual areas/roots:

```text
/engine
/system/settings
/system/extensions
/system/skills
/users
/projects
/memory
/artifacts
/tmp
/secrets
/events
```

These roots mirror the source-of-truth map in [`storage-placement.md`](storage-placement.md). Sub-roots such as `/engine/runtime` remain owned by their domain contracts.

`VirtualPath` rules:

```text
must be absolute
must begin with one known canonical root
normalization removes duplicate `/` and `.` segments
normalization rejects `..`
normalization rejects NUL/control characters
normalization never resolves to raw HostPath
```

### 7.3 Scoped aliases

V1 scoped aliases:

```text
/project
/workspace
/memory
/extension/config
/extension/state
/extension/cache
/tmp
/artifacts
```

`ScopedPath` rules:

```text
must be absolute within scoped namespace
must begin with an alias visible in the current MountView
must not contain `..`
must not contain NUL/control characters
must not look like a raw host path or URL
```

Rejected as `ScopedPath`:

```text
../../secret
/workspace/../../system/extensions/other
/Users/alice/project
C:\Users\alice\project
file:///etc/passwd
```

### 7.4 Host paths

`HostPath` is internal only.

Rules:

- no `Serialize` implementation by default
- never appears in model-visible output
- never appears in extension manifests
- debug output should not leak full host paths in hosted mode

---

## 8. Mount view contracts

A `MountView` describes what scoped paths an execution context can see.

```rust
pub struct MountView {
    pub mounts: Vec<MountGrant>,
}

pub struct MountGrant {
    pub alias: MountAlias,
    pub target: VirtualPath,
    pub permissions: MountPermissions,
}

pub struct MountPermissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub list: bool,
    pub execute: bool,
}
```

Rules:

- `MountView` resolution is fail-closed.
- If two aliases could match, longest alias wins.
- A child `MountView` must be a subset of the parent view unless an approved escalation creates a new grant.
- `/tmp` is invocation-local unless explicitly declared otherwise.
- `/extension/state` and `/extension/cache` resolve only for the current `extension_id`.

Required helper behavior:

```rust
fn resolve_scoped_path(view: &MountView, path: &ScopedPath) -> Result<VirtualPath, HostApiError>;
```

This function is pure lexical resolution. Backend symlink containment remains the filesystem backend's responsibility.

---

## 9. Principal and execution context

### 9.1 Principal

```rust
pub enum Principal {
    Tenant(TenantId),
    User(UserId),
    Agent(AgentId),
    Project(ProjectId),
    Mission(MissionId),
    Thread(ThreadId),
    Extension(ExtensionId),
    System,
}
```

### 9.2 Resource scope

Resource scope is a cascade, not a single owner.

```rust
pub struct ResourceScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
}
```

Rules:

- tenant and user are mandatory
- agent/project/mission/thread can be absent for host-system work, but absence must be explicit
- child invocation resource scope must preserve tenant, user, agent, and project from parent unless a trusted host workflow intentionally changes scope
- `_none` path partitions represent an intentionally absent optional scope, not the default local tenant/agent/project

#### Local single-user convention

Local or single-user deployments still normalize scope into concrete IDs so durable paths stay stable across backends. The recommended defaults are:

```text
tenant_id  = "default"
user_id    = the stable local user id, username, or hosted identity subject
agent_id   = Some("default") for the default local agent
project_id = Some("bootstrap") for the default local/bootstrap project
```

With those defaults, optional path partitions render as:

```text
/tenants/default/users/{user}/agents/default/projects/bootstrap/...
```

Use `agents/_none` or `projects/_none` only for deliberately unscoped/shared records. Do not use `_none` as a shorthand for the default single-agent or default-project experience; otherwise future additional agents or projects may accidentally share state that should have remained isolated.

The host API exposes these defaults as `LOCAL_DEFAULT_TENANT_ID`, `LOCAL_DEFAULT_AGENT_ID`, `LOCAL_DEFAULT_PROJECT_ID`, `ResourceScope::local_default(...)`, and `ExecutionContext::local_default(...)` so bootstrap/local callers do not hand-roll divergent defaults.

### 9.3 Execution context

```rust
pub struct ExecutionContext {
    pub invocation_id: InvocationId,
    pub correlation_id: CorrelationId,
    pub process_id: Option<ProcessId>,
    pub parent_process_id: Option<ProcessId>,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
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

Rules:

- `resource_scope.invocation_id == invocation_id`
- `resource_scope.tenant_id == tenant_id`
- `resource_scope.user_id == user_id`
- `resource_scope.agent_id == agent_id`
- `resource_scope.project_id == project_id`
- `process_id` may be absent for pure host calls or WASM invocations that are not process-backed
- every audit/event/budget decision must include `correlation_id`

---

## 10. Capabilities and grants

### 10.1 Effect kinds

```rust
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    ReadFilesystem,
    WriteFilesystem,
    DeleteFilesystem,
    Network,
    UseSecret,
    ExecuteCode,
    SpawnProcess,
    DispatchCapability,
    ModifyExtension,
    ModifyApproval,
    ModifyBudget,
    ExternalWrite,
    Financial,
}
```

### 10.2 Capability descriptor

A descriptor says what exists. It does not grant authority.

```rust
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub trust_ceiling: TrustClass,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub effects: Vec<EffectKind>,
    pub default_permission: PermissionMode,
    pub resource_profile: Option<ResourceProfile>,
}
```

### 10.3 Capability grants

A grant says who may use a capability and under what constraints.

```rust
pub struct CapabilityGrant {
    pub id: CapabilityGrantId,
    pub capability: CapabilityId,
    pub grantee: Principal,
    pub issued_by: Principal,
    pub constraints: GrantConstraints,
}

pub struct CapabilitySet {
    pub grants: Vec<CapabilityGrant>,
}
```

### 10.4 Grant constraints

```rust
pub struct GrantConstraints {
    pub allowed_effects: Vec<EffectKind>,
    pub mounts: MountView,
    pub network: NetworkPolicy,
    pub secrets: Vec<SecretHandle>,
    pub resource_ceiling: Option<ResourceCeiling>,
    pub expires_at: Option<Timestamp>,
    pub max_invocations: Option<u64>,
}
```

Rules:

- a child grant must not exceed parent constraints
- an expired grant is ignored
- a revoked grant is not represented in an active `CapabilitySet`
- declaration does not equal grant

---

## 11. Resource contracts

### 11.1 Estimates and usage

```rust
pub struct ResourceEstimate {
    pub usd: Option<Decimal>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub wall_clock_ms: Option<u64>,
    pub output_bytes: Option<u64>,
    pub network_egress_bytes: Option<u64>,
    pub process_count: Option<u32>,
    pub concurrency_slots: Option<u32>,
}

pub struct ResourceUsage {
    pub usd: Decimal,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_clock_ms: u64,
    pub output_bytes: u64,
    pub network_egress_bytes: u64,
    pub process_count: u32,
}

pub struct ResourceProfile {
    pub default_estimate: ResourceEstimate,
    pub hard_ceiling: Option<ResourceCeiling>,
}

pub struct ResourceCeiling {
    pub max_usd: Option<Decimal>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub sandbox: Option<SandboxQuota>,
}
```

### 11.2 Sandbox quotas

```rust
pub struct SandboxQuota {
    pub cpu_time_ms: Option<u64>,
    pub memory_bytes: Option<u64>,
    pub disk_bytes: Option<u64>,
    pub network_egress_bytes: Option<u64>,
    pub process_count: Option<u32>,
}
```

`ironclaw_host_api` defines these shapes. `ironclaw_resources`, `ironclaw_scripts`, `ironclaw_wasm`, and sandbox backends enforce them.

---

## 11a. Dispatch port contracts

`ironclaw_host_api` owns the neutral already-authorized dispatch port so caller-facing workflow crates can avoid depending on the concrete dispatcher implementation:

```rust
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub input: serde_json::Value,
}
pub struct CapabilityDispatchResult;
pub trait CapabilityDispatcher;
pub enum DispatchError;
pub enum RuntimeDispatchErrorKind;
```

Rules:

- `CapabilityDispatchRequest` is already authorized; grant checks and approvals happen before this boundary. Optional `mounts` and `resource_reservation` fields are prepared obligation effects, not new authority grants.
- `CapabilityDispatchResult` exposes normalized host facts: capability ID, provider, runtime, output, usage, and resource receipt.
- `DispatchError` uses stable control-plane variants for registry/routing failures and `RuntimeDispatchErrorKind` for WASM/Script/MCP failures.
- Runtime/backend detail strings, stderr, host paths, and secret-bearing messages must not cross this port.
- `ironclaw_dispatcher` implements the port; it does not own the port vocabulary.

---

## 12. Actions

```rust
pub enum Action {
    ReadFile {
        path: ScopedPath,
    },
    ListDir {
        path: ScopedPath,
    },
    WriteFile {
        path: ScopedPath,
        bytes: Option<u64>,
    },
    DeleteFile {
        path: ScopedPath,
    },

    Dispatch {
        capability: CapabilityId,
        estimated_resources: ResourceEstimate,
    },
    SpawnCapability {
        capability: CapabilityId,
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
    ExtensionLifecycle {
        extension_id: ExtensionId,
        operation: ExtensionLifecycleOperation,
    },
    EmitExternalEffect {
        effect: EffectKind,
    },
}
```

Rules:

- actions carry enough information for authorization, audit, and resource reservation
- actions do not contain raw secrets
- actions do not contain `HostPath`
- network target is validated before execution, not parsed ad hoc inside a runtime lane

---

## 13. Secrets

```rust
#[serde(rename_all = "snake_case")]
pub enum SecretUseMode {
    InjectIntoRequest,
    InjectIntoEnvironment,
    ReadRaw,
}
```

Rules:

- `ReadRaw` is denied by default or requires explicit high-risk approval
- `InjectIntoEnvironment` is only valid for strongly sandboxed process invocations
- secret values never appear in `Action`, `Decision`, audit records, logs, or model-visible output
- extension config stores secret handles only

---

## 14. Network

```rust
pub struct NetworkTarget {
    pub scheme: NetworkScheme,
    pub host: String,
    pub port: Option<u16>,
}

#[serde(rename_all = "snake_case")]
pub enum NetworkScheme {
    Http,
    Https,
}

#[serde(rename_all = "snake_case")]
pub enum NetworkMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

pub struct NetworkTargetPattern {
    pub scheme: Option<NetworkScheme>,
    pub host_pattern: String,
    pub port: Option<u16>,
}

pub struct NetworkPolicy {
    pub allowed_targets: Vec<NetworkTargetPattern>,
    pub deny_private_ip_ranges: bool,
    pub max_egress_bytes: Option<u64>,
}
```

`host_pattern` v0 should be intentionally simple: exact host or one leading wildcard label such as `*.github.com`. Do not support arbitrary regex in v0.

Rules:

- default `NetworkPolicy` denies outbound network
- private IP / localhost / metadata endpoints are denied unless explicitly allowed
- runtime lanes do not create raw network clients that bypass `ironclaw_network`

---

## 15. Approval and permission contracts

```rust
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

pub struct ApprovalRequest {
    pub id: ApprovalRequestId,
    pub correlation_id: CorrelationId,
    pub requested_by: Principal,
    pub action: Box<Action>,
    pub invocation_fingerprint: Option<InvocationFingerprint>,
    pub reason: String,
    pub reusable_scope: Option<ApprovalScope>,
}

pub struct InvocationFingerprint(String);

pub struct ApprovalScope {
    pub principal: Principal,
    pub action_pattern: ActionPattern,
    pub expires_at: Option<Timestamp>,
}

pub enum ActionPattern {
    ExactAction(Box<Action>),
    Capability(CapabilityId),
    PathPrefix { action_kind: FileActionKind, prefix: ScopedPath },
    NetworkTarget(NetworkTargetPattern),
}

#[serde(rename_all = "snake_case")]
pub enum FileActionKind {
    Read,
    List,
    Write,
    Delete,
}
```

Rules:

- approval matching is exact or policy-defined; never substring/heuristic authority
- reusable approvals must declare scope explicitly
- approvals are audited
- dispatch approval fingerprints are stable `sha256:` digests over scope, capability, estimate, and canonical JSON input; they must not store raw input payloads
- approval for one path/action/invocation fingerprint does not imply approval for broader paths/actions/inputs

---

## 16. Decisions and obligations

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

pub enum DenyReason {
    MissingGrant,
    InvalidPath,
    PathOutsideMount,
    UnknownCapability,
    UnknownSecret,
    NetworkDenied,
    BudgetDenied,
    ApprovalDenied,
    PolicyDenied,
    ResourceLimitExceeded,
    InternalInvariantViolation,
}

pub enum Obligation {
    AuditBefore,
    AuditAfter,
    RedactOutput,
    ReserveResources(ResourceReservationId),
    UseScopedMounts(MountView),
    InjectSecretOnce(SecretHandle),
    ApplyNetworkPolicy(NetworkPolicy),
    EnforceOutputLimit(u64),
}
```

Rules:

- most restrictive decision wins: `Deny > RequireApproval > Allow`
- an action is not complete until all `Allow` obligations are satisfied
- host services that do not yet implement obligation handlers must fail closed on non-empty obligations instead of silently ignoring them
- `Allow` without `AuditBefore/AuditAfter` is invalid for external side effects
- `Allow` without `ReserveResources` is invalid for costed/quota-limited work

---

## 17. Audit envelope

```rust
pub struct AuditEnvelope {
    pub event_id: AuditEventId,
    pub correlation_id: CorrelationId,
    pub stage: AuditStage,
    pub timestamp: Timestamp,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub agent_id: Option<AgentId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
    pub process_id: Option<ProcessId>,
    pub extension_id: Option<ExtensionId>,
    pub approval_request_id: Option<ApprovalRequestId>,

    pub action: ActionSummary,
    pub decision: DecisionSummary,
    pub result: Option<ActionResultSummary>,
}

pub struct ActionSummary {
    pub kind: String,
    pub target: Option<String>,
    pub effects: Vec<EffectKind>,
}

pub struct DecisionSummary {
    pub kind: String,
    pub reason: Option<DenyReason>,
    pub actor: Option<Principal>,
}

pub struct ActionResultSummary {
    pub success: bool,
    pub status: Option<String>,
    pub output_bytes: Option<u64>,
}

#[serde(rename_all = "snake_case")]
pub enum AuditStage {
    Before,
    After,
    Denied,
    ApprovalRequested,
    ApprovalResolved,
    ResourceReserved,
    ResourceReconciled,
    ResourceReleased,
}
```

Rules:

- audit payloads are redacted by construction
- raw secrets never appear
- raw host paths never appear
- every external side effect has before/after records or a denied record
- `correlation_id` joins all records for one logical action

---

## 18. Error contract

`HostApiError` is for contract validation failures, not service runtime failures.

```rust
pub enum HostApiError {
    InvalidId { kind: &'static str, value: String, reason: String },
    InvalidPath { value: String, reason: String },
    InvalidCapability { value: String, reason: String },
    InvalidMount { value: String, reason: String },
    InvalidNetworkTarget { value: String, reason: String },
    InvariantViolation { reason: String },
}
```

Service crates should wrap this in their own error types when needed.

---

## 19. Minimum implementation tests

The first `ironclaw_host_api` implementation is not accepted without tests for:

### IDs and names

- valid UUID-backed IDs serialize as strings
- invalid UUID-backed IDs fail to parse
- `ExtensionId` rejects slash, backslash, whitespace, NUL, uppercase, and `..`
- `CapabilityId` requires `<extension>.<capability>`
- scope IDs reject slash, backslash, NUL, controls, `.` and `..`

### Paths

- `ScopedPath` rejects raw host-looking paths
- `ScopedPath` rejects URL-looking paths
- `ScopedPath` rejects traversal
- `VirtualPath` requires a known root
- `resolve_scoped_path` chooses longest alias match
- `resolve_scoped_path` denies unknown alias
- child `MountView` helper denies broader permissions than parent

### Context/resource invariants

- `ExecutionContext` validation rejects mismatched `resource_scope.invocation_id`
- `ExecutionContext` validation rejects mismatched tenant/user between context and resource scope
- child resource scope preserves tenant/user from parent

### Action/decision serialization

- `Action` variants serialize with stable snake_case tags
- `Decision` variants serialize with stable snake_case tags
- `DenyReason` variants serialize with stable snake_case names
- `AuditStage` variants serialize with stable snake_case names

### Safety-by-construction

- `Action` cannot contain `HostPath`
- `AuditEnvelope` cannot contain raw host path fields
- `ApprovalRequest` carries an action, optional invocation fingerprint, and explicit reusable scope when reusable

---

## 20. Explicit non-goals for PR 1

Do not implement in `ironclaw_host_api`:

- real filesystem backend
- database schema
- budget ledger
- extension manifest discovery
- WASM invocation
- MCP client
- script runner
- auth/OAuth flows
- network client
- dispatcher builder
- agent loop
- gateway/TUI behavior

If an implementation detail requires one of those, stop and move it to the owning crate contract.

---

## 21. Acceptance criteria

The host API contract is ready when:

- `crates/ironclaw_host_api` builds as a standalone workspace crate
- no runtime/system-service crate dependency is introduced
- public types in this document exist or have a documented v0 substitute
- validation constructors exist for authority-bearing strings
- test coverage proves the minimum tests in this document
- architecture docs still name `ironclaw_host_api` as the first implementation crate

---

## 22. Implementation note

Prefer private fields plus validated constructors for authority-bearing strings:

```rust
pub struct ExtensionId(String);

impl ExtensionId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        // validate, then construct
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

Avoid public tuple fields for any type where invalid strings would break authorization or path invariants.


---

## Contract freeze addendum — global scope model (2026-04-25)

The global Reborn scope model must preserve `AgentId` as a first-class optional scope.

Required scope-bearing contracts should be updated to include:

```rust
pub struct AgentId(pub String);

pub struct ResourceScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub agent_id: Option<AgentId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
}

pub struct ExecutionContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub agent_id: Option<AgentId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub process_id: Option<ProcessId>,
    pub invocation_id: InvocationId,
    pub correlation_id: CorrelationId,
    // ...runtime/trust/grants/mounts/resources...
}
```

`AgentId` exists for production parity with current workspace memory partitioning. Every contract that persists or emits scoped user/project state must either carry `agent_id` or explicitly document why it is not agent-scoped.

Required propagation targets:

- memory documents/search/versions/layers;
- settings overrides when agent-scoped settings are allowed;
- events/audit/projections;
- resources/quotas;
- approvals/leases/run-state;
- process records/results/output refs;
- network/provider/secret usage records when tied to an agent invocation.
