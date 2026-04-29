# IronClaw Reborn events and audit contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_events`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/live-vertical-slice.md`

---

## 1. Purpose

`ironclaw_events` defines two separate observability surfaces:

```text
Runtime/process events -> RuntimeEvent + EventSink
Control-plane audit    -> AuditEnvelope + AuditSink
```

The split is intentional. Runtime events describe already-authorized dispatch and process lifecycle transitions. Approval resolution is host control-plane audit and is recorded as `AuditEnvelope { stage: ApprovalResolved, ... }`, not as a runtime event kind.

Both surfaces carry typed scope metadata and must not contain raw host paths, raw secrets, raw request payloads, approval reasons, invocation fingerprints, lease IDs, lease contents, or runtime output. Runtime `error_kind` fields are constrained to short classification strings; unsafe detail-like values are collapsed to `Unclassified`.

---

## 2. Runtime event shape

```rust
pub struct RuntimeEvent {
    pub event_id: RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: RuntimeEventKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
}
```

Current runtime/process event kinds:

```rust
pub enum RuntimeEventKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
}
```

Approval-specific runtime event kinds are deliberately absent. Approval resolution belongs to the audit envelope contract.

---

## 3. Audit sink shape

`ironclaw_events` provides an async `AuditSink` for control-plane audit records:

```rust
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError>;
}
```

V1 provides:

| Sink | Purpose |
| --- | --- |
| `InMemoryAuditSink` | Tests, demos, and live control-plane audit capture |
| `JsonlAuditSink<F: RootFilesystem>` | Durable JSONL audit history under a `VirtualPath` |

Approval resolution emits `AuditEnvelope::approval_resolved(...)` with:

```text
AuditStage::ApprovalResolved
ResourceScope fields
CorrelationId
ApprovalRequestId
optional ExtensionId for extension-requested approvals
ActionSummary
DecisionSummary { kind: "approved" | "denied", actor: Some(resolver principal), ... }
```

The audit record does not include the approval reason, replay input, invocation fingerprint, lease ID, lease contents, raw host paths, secret values, or runtime output. Audit sink failures are best-effort observability and must not change approval resolution outcomes.

---

## 4. Runtime event sinks

V1 provides:

| Sink | Purpose |
| --- | --- |
| `InMemoryEventSink` | Tests, demos, and live progress capture |
| `JsonlEventSink<F: RootFilesystem>` | Durable JSONL runtime/process history under a `VirtualPath` |

`JsonlEventSink` writes through `RootFilesystem`, not raw host paths. It also supports `read_events()` for deterministic demo/test readback from JSONL. Runtime and process events use this sink; approval audit uses `JsonlAuditSink`.

---

## 5. JSONL durability semantics

`JsonlEventSink` and `JsonlAuditSink` are append-style JSONL persistence helpers over the Reborn filesystem contract.

They treat only a true missing log file as an empty history. Backend, mount, permission, UTF-8, or malformed JSONL failures are returned as errors and must not be silently converted to empty history. `emit`/`emit_audit` validate existing JSONL before appending, then use `RootFilesystem::append_file` for the new JSONL record. They must not append to, overwrite, or truncate an existing log after a read or parse failure.

Dispatchers and approval resolvers still treat sink failures as best-effort observability. The sink returns errors to the caller, and the owning runtime/control-plane service decides whether the error is outcome-affecting. Current dispatcher and approval resolver semantics ignore sink errors and preserve the original dispatch/approval outcome.

---

## 6. Tenant/user/agent-scoped log paths

Durable event and audit paths should be tenant/user/agent scoped unless the caller is intentionally writing an admin-scoped aggregate log.

The events crate exposes helpers:

```rust
scoped_runtime_event_log_path(&scope, "reborn-demo.jsonl")
scoped_audit_log_path(&scope, "approval-audit.jsonl")
```

These produce paths such as:

```text
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/events/runtime/reborn-demo.jsonl
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/events/audit/approval-audit.jsonl
```

The helpers reject traversal-style names and require a simple `.jsonl` file name. Global paths under `/engine/events/...` should be treated as explicit admin/aggregate logs, not default runtime paths.

---

## 7. Dispatcher events

`RuntimeDispatcher::dispatch_json` emits:

Successful WASM/Script/MCP dispatch:

```text
dispatch_requested
runtime_selected
dispatch_succeeded
```

Preflight or runtime failure:

```text
dispatch_requested
dispatch_failed
```

`MissingRuntimeBackend`, unknown capability, runtime mismatch, unsupported runtime, and runtime execution failures all emit a failed event without leaking internal paths or secret values.

Runtime dispatcher event emission is best-effort observability. If the configured `EventSink` fails, the dispatcher ignores that sink error and still returns the original dispatch success or original dispatch failure.

---

## 8. Process lifecycle events

`ironclaw_processes::EventingProcessStore` can emit lifecycle events around successful process state transitions:

```text
start    -> process_started
complete -> process_completed
fail     -> process_failed
kill     -> process_killed
```

Each process event carries:

```text
ResourceScope
CapabilityId
provider ExtensionId
RuntimeKind
ProcessId
optional sanitized error_kind for process_failed
```

Process event emission is observability for this slice. It is deliberately outside `ironclaw_dispatcher`, so dispatcher remains process-blind and continues to route only already-authorized runtime dispatch requests.

---

## 9. Non-goals

This contract does not implement:

- global event bus fanout
- SSE/WebSocket reconnect semantics
- projection reducers
- full audit retention policy
- cryptographic audit integrity
- event subscription authorization
- transcript/job persistence
- durable process event projections beyond JSONL sinks

Those belong to later event/projection/audit slices.


---

## Contract freeze addendum — V1 stream scope (2026-04-25)

The event system must support durable append-log streams with replay cursors for SSE/WebSocket clients. Event records must carry tenant/user/project/agent/process/thread/invocation scope where relevant, and stream authorization must be checked on replay as well as live subscription.

Runtime/process/control-plane events remain redacted metadata. User-visible transcript/reply content is a separate durable product record and is not a license to include raw secrets, host paths, approval reasons, or unapproved runtime payloads in low-level events.
