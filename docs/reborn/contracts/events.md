# IronClaw Reborn events contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_events`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/dispatcher.md`, `docs/reborn/contracts/live-vertical-slice.md`

---

## 1. Purpose

`ironclaw_events` defines the first runtime/process event vocabulary and sink interfaces for Reborn. The V1 slices cover dispatcher-level observability and process lifecycle observability:

```text
dispatch requested
runtime selected
dispatch succeeded
dispatch failed
process started
process completed
process failed
process killed
```

Events carry typed scope/capability/runtime/process metadata. They must not contain raw host paths, raw secrets, or unredacted request payloads. Event `error_kind` fields are constrained to short classification strings; unsafe detail-like values are collapsed to `Unclassified`.

---

## 2. Current event shape

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

`MissingRuntimeBackend`, unknown capability, runtime mismatch, unsupported runtime, and runtime execution failures all emit a failed event without leaking internal paths or secret values.

Runtime dispatcher event emission is best-effort observability. If the configured `EventSink` fails, the dispatcher ignores that sink error and still returns the original dispatch success or original dispatch failure. Event sink outages must not turn successful capability calls into failures or mask runtime/control-plane errors.

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

Process event emission is observability for this slice. It is deliberately outside `ironclaw_dispatcher`, so dispatcher remains process-blind and continues to route only already-authorized runtime dispatch requests.

---

## 7. Non-goals

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
