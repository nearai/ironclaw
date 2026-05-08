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
    pub resource_reservation_id: Option<ResourceReservationId>, // store-assigned only
    pub error_kind: Option<String>,
}
```

The record always carries tenant/user/agent scope and capability identity so lifecycle, accounting, audit, and future runtime boundaries can be traced back to the same host authority envelope.

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

`ProcessStart.resource_reservation_id` is an internal store-assigned channel. Callers must not pre-fill it. `ResourceManagedProcessStore::start` rejects caller-supplied reservation IDs before persisting any process record so forged reservation IDs cannot bypass `ResourceGovernor::reserve`. The wrapper also tracks the reservations it created per process and refuses `complete`, `fail`, or `kill` cleanup for reservation IDs it did not create for that process.

`ProcessHost` is the current host-facing lifecycle API layered over `ProcessStore`:

```rust
async fn status(scope, process_id) -> Result<Option<ProcessRecord>>;
async fn kill(scope, process_id) -> Result<ProcessRecord>;
async fn await_process(scope, process_id) -> Result<ProcessExit>;
async fn subscribe(scope, process_id) -> Result<ProcessSubscription>;
async fn result(scope, process_id) -> Result<Option<ProcessResultRecord>>;
async fn output(scope, process_id) -> Result<Option<Value>>;
async fn await_result(scope, process_id) -> Result<ProcessResultRecord>;
```

`status` preserves tenant/user isolation by returning `None` for out-of-scope records. `kill` delegates to the scoped store transition and signals cooperative cancellation only after a scoped kill succeeds. `await_process` polls the scoped current-state store until the record reaches `Completed`, `Failed`, or `Killed`, then returns a terminal `ProcessExit`. `subscribe` returns a scoped current-state subscription whose first `next()` yields the current record, whose later `next()` calls yield status changes, and whose terminal record is emitted once before returning `None`. `result` and `await_result` read terminal output/error metadata from a scoped `ProcessResultStore`; `output` resolves inline or referenced JSON output through the same scoped store. Missing or out-of-scope records fail closed with `UnknownProcess`.

The V1 subscription is intentionally scoped and current-state based. It does not expose raw process input/output, host paths, or cross-tenant existence information, and it does not require `CapabilityHost` or `ironclaw_dispatcher` to own process lifecycle mechanics.

`ProcessServices` is a composition helper that wires the process store, result store, and cancellation registry together so `ProcessHost` and `BackgroundProcessManager` share the same lifecycle/result/cancellation state:

```rust
let services = ProcessServices::in_memory();
let host = services.host();
let manager = services.background_manager(executor);
```

It also supports filesystem-backed composition from a shared filesystem handle. `CapabilityHost::with_process_services(...)` can derive its spawn manager from this same bundle, while callers still use `services.host()` for lifecycle/result/output operations. This helper is convenience wiring only; it does not move process lifecycle into `CapabilityHost`, `ironclaw_dispatcher`, or any runtime lane.

`BackgroundProcessManager` composes a `ProcessStore` and `ProcessExecutor`:

```text
start ProcessRecord as Running
  -> spawn background executor task
  -> executor success: complete(scope, process_id)
  -> executor failure: fail(scope, process_id, error_kind)
```

The executor receives a redaction-friendly `ProcessExecutionRequest` containing process identity, scope, target capability, estimate, raw input, and a `ProcessCancellationToken`. When the process record already carries a process-owned reservation ID, `BackgroundProcessManager` sends a zero/default dispatch estimate so a runtime-backed process does not reserve the same process estimate twice. If configured with a `ProcessResultStore`, it records `ProcessExecutionResult.output` after a successful `complete` transition, records sanitized failure kind after a successful `fail` transition, and does not overwrite a `Killed` result after late executor completion.

`ProcessResultRecord` is separate from `ProcessRecord`:

```rust
pub struct ProcessResultRecord {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub status: ProcessStatus,
    pub output: Option<Value>,
    pub output_ref: Option<VirtualPath>,
    pub error_kind: Option<String>,
}
```

In-memory/dev result stores may keep small JSON output inline. Filesystem-backed process results write successful JSON output to scoped output artifact paths and store only `output_ref` in the result record, keeping lifecycle/result metadata small and easier to redact. This is still not a streaming/binary output system; later slices should generalize these refs for large, streaming, binary, or sensitive outputs.

`ProcessCancellationRegistry` is optional wiring shared by `BackgroundProcessManager` and `ProcessHost`. The manager registers a token under tenant/user/process scope before starting executor work. `ProcessHost::kill` removes and signals the matching token only after the scoped store kill succeeds. Cross-tenant or cross-user kill attempts therefore cannot cancel another tenant/user's running executor even if they know a process UUID. Executor cancellation is cooperative: runtime adapters must observe `ProcessExecutionRequest.cancellation` and stop themselves.

`FilesystemProcessStore::from_arc(...)` provides an owned store handle for detached background managers. The filesystem store serializes start/status writes within a store instance; production DB/object-store implementations should use compare-and-swap or transactional updates for cross-process terminal-state protection.

`ResourceManagedProcessStore` wraps any `ProcessStore` and owns process reservation cleanup:

```text
start
  -> ResourceGovernor::reserve(scope, estimate)
  -> attach resource_reservation_id to ProcessStart
  -> inner.start(...)
  -> on inner start failure: release reservation

complete
  -> inner.complete(...)
  -> reconcile reservation with configured completion usage

fail / kill
  -> inner.fail(...) / inner.kill(...)
  -> release reservation without recording usage
```

Resource denial fails before process record creation. Caller-supplied reservation IDs are rejected before process record creation. The wrapper verifies that the inner store preserves the reservation ID it created and releases the reservation if start fails. The wrapper is deliberately below `CapabilityHost` and above concrete stores so resource ownership can compose with in-memory, filesystem, eventing, and future durable stores without making `ironclaw_dispatcher` process-aware.

`EventingProcessStore` wraps any `ProcessStore` and emits best-effort lifecycle events after successful state transitions:

```text
start    -> process_started
complete -> process_completed
fail     -> process_failed
kill     -> process_killed
```

These events include tenant/user/agent `ResourceScope`, `CapabilityId`, provider `ExtensionId`, `RuntimeKind`, and `ProcessId`. The wrapper does not make `ironclaw_dispatcher` process-aware; process observability stays in the process lifecycle service.

`start` rejects duplicate process IDs within the same tenant/user/agent partition. Callers must transition existing records instead of overwriting lifecycle state. `complete`, `fail`, and `kill` only transition from `Running`; late executor completions after `kill` are ignored by the background manager because the store rejects the terminal-state overwrite. Because event emission happens after successful transitions, a killed process does not emit a misleading late `process_completed` event when the background executor finishes after kill.

---

## 5. Tenant/user/agent partitioning

Process records and result records are tenant/user/agent scoped. The filesystem-backed stores write through `RootFilesystem` under:

```text
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/processes/{process_id}.json
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/process-results/{process_id}.json
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/process-outputs/{process_id}/output.json
```

Cross-tenant, cross-user, and cross-agent reads return `None`, empty lists, or `UnknownProcess`; they must not reveal that another tenant/user/agent has a matching process UUID.

---

## 6. Current non-goals

This slice does not implement:

- direct WASM/Script/MCP process loops inside `ironclaw_processes`; runtime work is delegated through `ProcessExecutor`
- dynamic executor-reported actual resource usage; completion reconciliation currently uses configured/default usage
- forced/preemptive cancellation of uncooperative executor tasks
- generalized artifact references for large, streaming, binary, or sensitive outputs beyond the current JSON output file
- streaming output APIs
- durable subscription cursors or process event projection/read APIs beyond the shared event sink/current-state subscription
- process tree queries beyond parent process ID storage
- durable resource ledger beyond the configured `ResourceGovernor` implementation
- approval resume for `Action::SpawnCapability`

Those should be layered on this capability-backed process record and manager boundary.


---

## Contract freeze addendum — V1 process/output API (2026-04-25)

The V1 process contract includes:

```text
status
kill/cancel
await completion
subscribe to lifecycle/progress events
result record
output refs
streaming output/progress through durable events
```

Large or binary outputs are represented by artifact refs, not embedded in process records or event payloads.

Process scope must include tenant/user/project/agent where available. Process events and result records must not leak raw input, raw output beyond allowed output refs, host paths, secret material, or backend detail strings.

Terminal-like raw stdout/stderr streams, forced abort handles, and advanced binary streaming are V2 unless implemented behind the same redacted event/artifact-ref contract.
