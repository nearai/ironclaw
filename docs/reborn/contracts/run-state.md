# IronClaw Reborn run-state contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_run_state`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capabilities.md`

---

## 1. Purpose

`ironclaw_run_state` stores the current lifecycle state for host-managed invocations.

It is distinct from runtime events:

```text
events      -> append-only history of what happened
run state   -> current answer to “what is this invocation doing or waiting on?”
```

The first slice is intentionally small and in-memory. It exists so the capability host can represent blocked approval and terminal completion/failure states without making the dispatcher own workflow state.

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

## 3. Store contract

The V1 store API is current-state oriented:

```rust
pub trait RunStateStore {
    fn start(&self, start: RunStart) -> RunRecord;
    fn block_approval(&self, invocation_id, approval) -> Result<RunRecord, RunStateError>;
    fn block_auth(&self, invocation_id, error_kind) -> Result<RunRecord, RunStateError>;
    fn complete(&self, invocation_id) -> Result<RunRecord, RunStateError>;
    fn fail(&self, invocation_id, error_kind) -> Result<RunRecord, RunStateError>;
    fn get(&self, invocation_id) -> Option<RunRecord>;
}
```

`InMemoryRunStateStore` is provided for tests and the live slice. Durable run-state persistence can be added later through the filesystem or database service.

---

## 4. Capability host integration

`CapabilityHost` may be configured with a run-state store:

```rust
CapabilityHost::new(&registry, &dispatcher, &authorizer)
    .with_run_state(&run_state)
```

When configured, `invoke_json` records:

```text
start -> Running
Decision::RequireApproval -> BlockedApproval
Decision::Deny -> Failed(error_kind = AuthorizationDenied)
dispatch success -> Completed
dispatch failure -> Failed(error_kind = Dispatch)
```

The dispatcher remains run-state-unaware. It still routes already-authorized dispatches only.

---

## 5. Non-goals

This slice does not implement:

- durable run-state storage
- append-only transition history
- approval resolution/resume
- auth/OAuth blocking semantics beyond reserving `BlockedAuth`
- cancellation
- retries
- parent/child run trees
- websocket/SSE projections

Those should be follow-on slices built on this current-state contract.
