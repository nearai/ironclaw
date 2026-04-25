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

`BlockedAuth` is reserved for future auth/OAuth/secret-auth flows. A grant denial is currently terminal `Failed`, not `BlockedAuth`.

---

## 3. Store contracts

The run-state API is current-state oriented and async so durable implementations can use the host filesystem abstraction:

```rust
pub trait RunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError>;
    async fn block_approval(&self, invocation_id, approval) -> Result<RunRecord, RunStateError>;
    async fn block_auth(&self, invocation_id, error_kind) -> Result<RunRecord, RunStateError>;
    async fn complete(&self, invocation_id) -> Result<RunRecord, RunStateError>;
    async fn fail(&self, invocation_id, error_kind) -> Result<RunRecord, RunStateError>;
    async fn get(&self, invocation_id) -> Result<Option<RunRecord>, RunStateError>;
    async fn records(&self) -> Result<Vec<RunRecord>, RunStateError>;
}
```

Approval requests have a separate store because they are durable objects that need independent resolution later:

```rust
pub trait ApprovalRequestStore {
    async fn save_pending(&self, request: ApprovalRequest) -> Result<ApprovalRecord, RunStateError>;
    async fn get(&self, request_id) -> Result<Option<ApprovalRecord>, RunStateError>;
    async fn records(&self) -> Result<Vec<ApprovalRecord>, RunStateError>;
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

## 4. Filesystem persistence

Filesystem-backed stores persist through `ironclaw_filesystem::RootFilesystem`, not direct host paths or database APIs:

```text
/engine/runs/{invocation_id}.json
/engine/approvals/{approval_request_id}.json
```

This is intentional. Production can later back `/engine` with a DB-backed filesystem/document-store implementation while Reborn service crates continue depending on host storage traits instead of Postgres/libSQL internals.

The filesystem store is durable current-state storage. It is not a transition log; runtime events remain the append-only history lane.

---

## 5. Capability host integration

`CapabilityHost` may be configured with run-state and approval stores:

```rust
CapabilityHost::new(&registry, &dispatcher, &authorizer)
    .with_run_state(&run_state)
    .with_approval_requests(&approval_requests)
```

When configured, `invoke_json` records:

```text
start -> Running
Decision::RequireApproval -> save pending ApprovalRecord + BlockedApproval
Decision::Deny -> Failed(error_kind = AuthorizationDenied)
dispatch success -> Completed
dispatch failure -> Failed(error_kind = Dispatch)
```

The dispatcher remains run-state-unaware. It still routes already-authorized dispatches only.

---

## 6. Non-goals

This slice does not implement:

- approval resolution/resume
- grant/lease issuance from approved requests
- append-only transition history
- atomic transactions across run-state and approval stores
- auth/OAuth blocking semantics beyond reserving `BlockedAuth`
- cancellation
- retries
- parent/child run trees
- websocket/SSE projections

Those should be follow-on slices built on this current-state and approval-request contract.
