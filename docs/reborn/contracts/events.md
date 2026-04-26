# IronClaw Reborn events contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_events`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/live-vertical-slice.md`

---

## 1. Purpose

`ironclaw_events` defines the first runtime/process/control-plane event vocabulary and sink interfaces for Reborn. The V1 slices cover dispatcher-level observability, process lifecycle observability, and approval-resolution audit metadata:

```text
dispatch requested
runtime selected
dispatch succeeded
dispatch failed
process started
process completed
process failed
process killed
approval approved
approval denied
```

Events carry typed scope/capability/runtime/process/approval metadata. They must not contain raw host paths, raw secrets, unredacted request payloads, approval reasons, invocation fingerprints, or lease contents. Event `error_kind` fields use the shared host-safe `ErrorKind` contract; unsafe detail-like values are collapsed to `Unclassified`.

---

## 2. Current event shape

```rust
pub struct RuntimeEvent {
    pub event_id: RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: RuntimeEventKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
}
```

Current event kinds:

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
    ApprovalApproved,
    ApprovalDenied,
}
```

---

## 3. Sinks

V1 provides two sinks:

| Sink | Purpose |
| --- | --- |
| `InMemoryEventSink` | Tests, demos, and live progress capture |
| `JsonlEventSink<F: RootFilesystem>` | Durable JSONL runtime history under a `VirtualPath` |

`JsonlEventSink` writes through `RootFilesystem`, not raw host paths. It also supports `read_events()` for deterministic demo/test readback from JSONL. It is a minimal durable history sink, not the final audit store, replay service, or stream fanout implementation.

---

## 4. Durable event path in the live slice

The live vertical slice mounts an explicit `/engine` root and persists demo dispatch events at:

```text
/engine/events/reborn-demo.jsonl
```

The path is a `VirtualPath`; runtime code and guests still do not receive raw host paths.

---

## 5. Dispatcher events

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

`MissingRuntimeBackend`, unknown capability, runtime mismatch, unsupported runtime, and runtime execution failures all emit a failed event without leaking internal paths or secret values. Event sink failures are best-effort observability failures; they must not fail an otherwise successful dispatch or overwrite the real dispatch failure kind.

The live vertical slice currently emits nine events for its three successful lanes: WASM, Script, and MCP.

---

## 6. Process lifecycle events

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

Process event emission is observability for this slice. It is deliberately outside `ironclaw_dispatcher`, so dispatcher remains process-blind and continues to route only already-authorized runtime dispatch requests. Process records and lifecycle events use the same sanitized `ErrorKind` contract for status/error classifications.

---

## 7. Approval resolution audit events

`ironclaw_approvals::ApprovalResolver` can be configured with an optional `EventSink`. After a successful dispatch approval or denial state transition, it emits:

```text
approve_dispatch success -> approval_approved
deny success             -> approval_denied
```

Each approval audit event carries only:

```text
ResourceScope
CapabilityId
ApprovalRequestId
```

It does not include the approval reason, replay input, invocation fingerprint, lease ID, lease contents, approver-specific secret material, or runtime output. Approval event emission is best-effort observability: sink failures are ignored and must not change approval resolution outcomes.

Approval events are emitted by `ironclaw_approvals`, not by `ironclaw_dispatcher`, so the dispatcher remains authorization-, approval-, and state-blind.

---

## 8. Non-goals

This contract does not implement:

- global event bus fanout
- SSE/WebSocket reconnect semantics
- projection reducers
- full audit retention policy
- cryptographic audit integrity
- event subscription authorization
- transcript/job persistence
- durable process event projections beyond the shared JSONL sink

Those belong to later event/projection/audit slices.
