# IronClaw Reborn run-state contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_run_state`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/filesystem.md`, `docs/reborn/contracts/capabilities.md`

---

## 1. Purpose

`ironclaw_run_state` stores the current lifecycle state for host-managed invocations and the pending approval requests that can block them.

It is distinct from runtime events:

```text
events      -> append-only history of what happened
run state   -> current answer to “what is this invocation doing or waiting on?”
approvals   -> durable request objects that a human/policy service can resolve later
```

This crate lives in the host control plane. It is not part of WASM, Script, MCP, or dispatcher runtime execution.

Multi-tenancy is part of the contract. Records are keyed by invocation/request IDs but always read, listed, and transitioned through a tenant/user/agent `ResourceScope` partition.

---

## 2. Current status model

```rust
pub enum RunStatus {
    Running,
    BlockedApproval,
    BlockedAuth,
    Completed,
    Failed,
}
```

Current records include:

```rust
pub struct RunRecord {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub status: RunStatus,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub error_kind: Option<String>,
}
```

Approval records also carry scope:

```rust
pub struct ApprovalRecord {
    pub scope: ResourceScope,
    pub request: ApprovalRequest,
    pub status: ApprovalStatus,
}
```

`BlockedAuth` is reserved for future auth/OAuth/secret-auth flows. A grant denial is currently terminal `Failed`, not `BlockedAuth`.

---

## 3. Store contracts

The run-state API is current-state oriented and async so durable implementations can use the host filesystem abstraction.

Every read, list, and mutation after `start` requires a `ResourceScope`. `start` creates a new invocation record and must fail if the same tenant/user/agent/invocation already exists; callers must use explicit resume/transition APIs rather than overwriting current state. Stored `error_kind` values use the shared sanitized `ErrorKind` contract so current-state APIs do not expose detail-like runtime strings.


```rust
pub trait RunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError>;
    async fn block_approval(&self, scope, invocation_id, approval) -> Result<RunRecord, RunStateError>;
    async fn block_auth(&self, scope, invocation_id, error_kind) -> Result<RunRecord, RunStateError>;
    async fn complete(&self, scope, invocation_id) -> Result<RunRecord, RunStateError>;
    async fn fail(&self, scope, invocation_id, error_kind) -> Result<RunRecord, RunStateError>;
    async fn get(&self, scope, invocation_id) -> Result<Option<RunRecord>, RunStateError>;
    async fn records_for_scope(&self, scope) -> Result<Vec<RunRecord>, RunStateError>;
}
```

Approval requests have a separate store because they are durable objects that need independent resolution later:

```rust
pub trait ApprovalRequestStore {
    async fn save_pending(&self, scope, request) -> Result<ApprovalRecord, RunStateError>;
    async fn get(&self, scope, request_id) -> Result<Option<ApprovalRecord>, RunStateError>;
    async fn approve(&self, scope, request_id) -> Result<ApprovalRecord, RunStateError>;
    async fn deny(&self, scope, request_id) -> Result<ApprovalRecord, RunStateError>;
    async fn records_for_scope(&self, scope) -> Result<Vec<ApprovalRecord>, RunStateError>;
}
```

Current implementations:

```text
InMemoryRunStateStore
InMemoryApprovalRequestStore
FilesystemRunStateStore
FilesystemApprovalRequestStore
```

---

## 4. Tenant/user/agent partitioning

Stores partition durable data by tenant, user, and agent from `ResourceScope`:

```text
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/runs/{invocation_id}.json
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/approvals/{approval_request_id}.json
```

The full `ResourceScope` remains inside each record for project/agent/mission/thread/invocation metadata. The first hard isolation boundary is tenant/user/agent; later projection/index layers can add project/thread views without weakening tenant/user/agent partitioning.

Store APIs hide cross-tenant, cross-user, and cross-agent records by returning `None`, an empty list, `UnknownInvocation`, or `UnknownApprovalRequest`. They must not expose whether another tenant/user/agent has a matching UUID. This applies to in-memory stores too: test/dev backends use tenant/user/agent/UUID composite keys rather than UUID-only maps.

---

## 5. Filesystem persistence

Filesystem-backed stores persist through `ironclaw_filesystem::RootFilesystem`, not direct host paths or database APIs.

This is intentional. Production can later back `/engine` with a DB-backed filesystem/document-store implementation while Reborn service crates continue depending on host storage traits instead of Postgres/libSQL internals.

The filesystem store is durable current-state storage. It is not a transition log; runtime events remain the append-only history lane.

---

## 6. Capability host integration

`CapabilityHost` may be configured with run-state and approval stores:

```rust
CapabilityHost::new(&registry, &dispatcher, &authorizer)
    .with_run_state(&run_state)
    .with_approval_requests(&approval_requests)
```

When configured, `invoke_json` records under the caller's `ExecutionContext.resource_scope`:

```text
start -> Running
Decision::RequireApproval -> save pending ApprovalRecord + BlockedApproval
Decision::RequireApproval with incoherent store wiring -> Failed(error_kind = ApprovalStoreMissing)
Decision::Deny -> Failed(error_kind = AuthorizationDenied)
dispatch success -> Completed
dispatch failure -> Failed(error_kind = Dispatch)
spawn process record created -> Completed
spawn process creation failed -> Failed(error_kind = ProcessSpawn)
```

For `spawn_json`, run state tracks the start request lifecycle only. Long-running/background process lifecycle belongs to `ironclaw_processes::ProcessStore` after the start workflow returns a `ProcessId`.

`resume_json` continues a `BlockedApproval` run only after loading an approved request and matching lease under the same tenant/user/agent/invocation scope:

```text
Approved + matching fingerprint + active lease -> claim lease -> dispatch -> consume lease -> Completed
Denied/Expired/Pending approval -> Failed(error_kind = ApprovalDenied/ApprovalExpired/ApprovalPending)
fingerprint mismatch -> Failed(error_kind = InvocationFingerprintMismatch)
missing lease -> Failed(error_kind = ApprovalLeaseMissing)
failed lease claim -> Failed(error_kind = ApprovalLeaseClaim)
dispatch failure -> Failed(error_kind = Dispatch)
```

The dispatcher remains run-state-unaware. It still routes already-authorized dispatches only.

---

## 7. Non-goals

This slice does not implement:

- durable grant/lease persistence
- append-only transition history
- atomic transactions across run-state and approval stores
- project/thread secondary indexes
- auth/OAuth blocking semantics beyond reserving `BlockedAuth`
- cancellation
- retries
- parent/child run trees
- websocket/SSE projections

Those should be follow-on slices built on this scoped current-state and approval-request contract.


---

## Contract freeze addendum — scope and history split (2026-04-25)

Run-state remains current-state storage, not transition history. Durable history and UI replay are owned by the event/audit append log and projections.

Run-state records must include `agent_id` where the owning execution context carries one, and must preserve exact invocation identity for V1 approval/resume flows.
