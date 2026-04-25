# IronClaw Reborn process lifecycle contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_processes`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/filesystem.md`

---

## 1. Purpose

`ironclaw_processes` owns host-tracked background capability lifecycle state.

It is intentionally below `CapabilityHost`:

```text
CapabilityHost::spawn_json(...)
  -> validates scope and authorization
  -> selects a declared capability descriptor
  -> asks ProcessManager to create a process record

ironclaw_processes
  -> stores process identity and lifecycle
  -> optionally starts background execution through ProcessExecutor
  -> exposes status transitions such as complete/fail/kill
```

It does not decide whether a caller may spawn a capability. Authorization remains in `ironclaw_authorization`, and caller-facing workflow remains in `ironclaw_capabilities`.

---

## 2. Capability-backed process records

A process is a tracked runtime instance of a declared capability, not a raw host process escape:

```rust
pub struct ProcessRecord {
    pub process_id: ProcessId,
    pub parent_process_id: Option<ProcessId>,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub status: ProcessStatus,
    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub estimated_resources: ResourceEstimate,
    pub resource_reservation_id: Option<ResourceReservationId>,
    pub error_kind: Option<String>,
}
```

The record always carries tenant/user scope and capability identity so lifecycle, accounting, audit, and future runtime boundaries can be traced back to the same host authority envelope.

---

## 3. Status model

The first slice keeps process status minimal:

```rust
pub enum ProcessStatus {
    Running,
    Completed,
    Failed,
    Killed,
}
```

`spawn_json` creates a `Running` process record. `BackgroundProcessManager` then drives `Running -> Completed` or `Running -> Failed` from the attached `ProcessExecutor`. Terminal states are protected: `Completed`, `Failed`, and `Killed` cannot be overwritten by a late background completion.

---

## 4. Store and manager contracts

`ProcessStore` is current-state storage for process lifecycle:

```rust
async fn start(ProcessStart) -> Result<ProcessRecord>;
async fn complete(scope, process_id) -> Result<ProcessRecord>;
async fn fail(scope, process_id, error_kind) -> Result<ProcessRecord>;
async fn kill(scope, process_id) -> Result<ProcessRecord>;
async fn get(scope, process_id) -> Result<Option<ProcessRecord>>;
async fn records_for_scope(scope) -> Result<Vec<ProcessRecord>>;
```

`ProcessManager::spawn` is the lower-level lifecycle mechanic used by `CapabilityHost`. It receives the spawn input in `ProcessStart` so runtime-backed managers can start work, but `ProcessRecord` does not persist raw input. The in-memory and filesystem stores implement the manager by recording a new `Running` process.

`BackgroundProcessManager` composes a `ProcessStore` and `ProcessExecutor`:

```text
start ProcessRecord as Running
  -> spawn background executor task
  -> executor success: complete(scope, process_id)
  -> executor failure: fail(scope, process_id, error_kind)
```

The executor receives a redaction-friendly `ProcessExecutionRequest` containing process identity, scope, target capability, estimate, and raw input. It returns `ProcessExecutionResult` for future output/event handling; this slice does not persist process output.

`FilesystemProcessStore::from_arc(...)` provides an owned store handle for detached background managers. The filesystem store serializes start/status writes within a store instance; production DB/object-store implementations should use compare-and-swap or transactional updates for cross-process terminal-state protection.

`start` rejects duplicate process IDs within the same tenant/user partition. Callers must transition existing records instead of overwriting lifecycle state. `complete`, `fail`, and `kill` only transition from `Running`; late executor completions after `kill` are ignored by the background manager because the store rejects the terminal-state overwrite.

---

## 5. Tenant/user partitioning

Process records are tenant/user scoped. The filesystem-backed store writes through `RootFilesystem` under:

```text
/engine/tenants/{tenant_id}/users/{user_id}/processes/{process_id}.json
```

Cross-tenant and cross-user reads return `None`, empty lists, or `UnknownProcess`; they must not reveal that another tenant/user has a matching process UUID.

---

## 6. Current non-goals

This slice does not implement:

- direct WASM/Script/MCP process loops inside `ironclaw_processes`; runtime work is delegated through `ProcessExecutor`
- cooperative cancellation/abort handles for running executor tasks
- `await`, `subscribe`, or streaming output APIs
- durable append-only process event history
- process tree queries beyond parent process ID storage
- resource reservation ownership/cleanup beyond the optional reservation ID field
- approval resume for `Action::SpawnCapability`

Those should be layered on this capability-backed process record and manager boundary.
