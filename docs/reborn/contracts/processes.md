# IronClaw Reborn process lifecycle contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_processes`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/filesystem.md`, `docs/reborn/contracts/events.md`

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
  -> optionally owns resource reservations through ResourceManagedProcessStore
  -> optionally emits process lifecycle events through EventingProcessStore
  -> exposes host-facing lifecycle APIs through ProcessHost
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
    pub error_kind: Option<ErrorKind>,
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

`spawn_json` creates a `Running` process record. `BackgroundProcessManager` then drives `Running -> Completed` or `Running -> Failed` from the attached `ProcessExecutor`. `ProcessHost::kill` drives `Running -> Killed` and, when configured with a shared `ProcessCancellationRegistry`, also signals the running executor's cooperative cancellation token. Terminal states are protected: `Completed`, `Failed`, and `Killed` cannot be overwritten by a late background completion.

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

`ProcessHost` is the current host-facing lifecycle API layered over `ProcessStore`:

```rust
async fn status(scope, process_id) -> Result<Option<ProcessRecord>>;
async fn kill(scope, process_id) -> Result<ProcessRecord>;
async fn await_process(scope, process_id) -> Result<ProcessExit>;
async fn subscribe(scope, process_id) -> Result<ProcessSubscription>;
```

`status` preserves tenant/user isolation by returning `None` for out-of-scope records. `kill` delegates to the scoped store transition and signals cooperative cancellation only after a scoped kill succeeds. `await_process` polls the scoped current-state store until the record reaches `Completed`, `Failed`, or `Killed`, then returns a terminal `ProcessExit`. `subscribe` returns a scoped current-state subscription whose first `next()` yields the current record, whose later `next()` calls yield status changes, and whose terminal record is emitted once before returning `None`. Missing or out-of-scope records fail closed with `UnknownProcess`.

The V1 subscription is intentionally scoped and current-state based. It does not expose raw process input/output, host paths, or cross-tenant existence information, and it does not require `CapabilityHost` or `ironclaw_dispatcher` to own process lifecycle mechanics.

`BackgroundProcessManager` composes a `ProcessStore` and `ProcessExecutor`:

```text
start ProcessRecord as Running
  -> spawn background executor task
  -> executor success: complete(scope, process_id)
  -> executor failure: fail(scope, process_id, error_kind)
```

The executor receives a redaction-friendly `ProcessExecutionRequest` containing process identity, scope, target capability, estimate, optional process-owned reservation ID, raw input, and a `ProcessCancellationToken`. When the process record already carries a process-owned reservation ID, `BackgroundProcessManager` sends a zero/default dispatch estimate so a runtime-backed process does not reserve the same process estimate twice. It returns `ProcessExecutionResult` for future output/event handling; this slice does not persist process output.

`ProcessCancellationRegistry` is optional wiring shared by `BackgroundProcessManager` and `ProcessHost`. The manager registers a token under tenant/user/process scope before starting executor work. `ProcessHost::kill` removes and signals the matching token only after the scoped store kill succeeds. Cross-tenant or cross-user kill attempts therefore cannot cancel another tenant/user's running executor even if they know a process UUID. Executor cancellation is cooperative: runtime adapters must observe `ProcessExecutionRequest.cancellation` and stop themselves.

`FilesystemProcessStore::from_arc(...)` provides an owned store handle for detached background managers. The filesystem store serializes start/status writes within a store instance; production DB/object-store implementations should use compare-and-swap or transactional updates for cross-process terminal-state protection.

`ResourceManagedProcessStore` wraps any `ProcessStore` and owns process reservation cleanup:

```text
start
  -> ResourceGovernor::reserve(scope, estimate)
  -> attach an internal non-forgeable process reservation handle to ProcessStart
  -> inner.start(...)
  -> on inner start failure: release reservation

complete
  -> inner.complete(...)
  -> reconcile reservation with configured completion usage

fail / kill
  -> inner.fail(...) / inner.kill(...)
  -> release reservation without recording usage
```

Resource denial fails before process record creation. Public callers create `ProcessStart` with `ProcessResourceReservation::none()`; only `ResourceManagedProcessStore` can attach a reserved handle. The wrapper verifies that the inner store preserves the reservation ID it created and releases the reservation if start fails. The wrapper is deliberately below `CapabilityHost` and above concrete stores so resource ownership can compose with in-memory, filesystem, eventing, and future durable stores without making `ironclaw_dispatcher` process-aware.

`EventingProcessStore` wraps any `ProcessStore` and emits best-effort lifecycle events after successful state transitions:

```text
start    -> process_started
complete -> process_completed
fail     -> process_failed
kill     -> process_killed
```

These events include tenant/user `ResourceScope`, `CapabilityId`, provider `ExtensionId`, `RuntimeKind`, and `ProcessId`. Error kinds are stored and emitted through the shared sanitized `ErrorKind` contract. The wrapper does not make `ironclaw_dispatcher` process-aware; process observability stays in the process lifecycle service.

`start` rejects duplicate process IDs within the same tenant/user partition. Callers must transition existing records instead of overwriting lifecycle state. `complete`, `fail`, and `kill` only transition from `Running`; late executor completions after `kill` are ignored by the background manager because the store rejects the terminal-state overwrite. Because event emission happens after successful transitions, a killed process does not emit a misleading late `process_completed` event when the background executor finishes after kill.

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
- dynamic executor-reported actual resource usage; completion reconciliation currently uses configured/default usage
- forced/preemptive cancellation of uncooperative executor tasks
- streaming output APIs
- durable subscription cursors or process event projection/read APIs beyond the shared event sink/current-state subscription
- process tree queries beyond parent process ID storage
- durable resource ledger beyond the configured `ResourceGovernor` implementation
- approval resume for `Action::SpawnCapability`

Those should be layered on this capability-backed process record and manager boundary.
