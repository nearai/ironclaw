# IronClaw Reborn events contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_events`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/kernel-dispatch.md`, `docs/reborn/contracts/live-vertical-slice.md`

---

## 1. Purpose

`ironclaw_events` defines the first runtime event vocabulary and sink interfaces for Reborn. The V1 slice focuses on dispatcher-level observability:

```text
dispatch requested
runtime selected
dispatch succeeded
dispatch failed
```

Events carry typed scope/capability/runtime metadata. They must not contain raw host paths, raw secrets, or unredacted request payloads.

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
}
```

---

## 3. Sinks

V1 provides two sinks:

| Sink | Purpose |
| --- | --- |
| `InMemoryEventSink` | Tests, demos, and live progress capture |
| `JsonlEventSink<F: RootFilesystem>` | Durable JSONL runtime history under a `VirtualPath` |

`JsonlEventSink` writes through `RootFilesystem`, not raw host paths. It is a minimal durable history sink, not the final audit store, replay service, or stream fanout implementation.

---

## 4. Kernel dispatch events

`RuntimeDispatcher::dispatch_json` emits:

Successful WASM/Script dispatch:

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

---

## 5. Non-goals

This contract does not implement:

- global event bus fanout
- SSE/WebSocket reconnect semantics
- projection reducers
- full audit retention policy
- cryptographic audit integrity
- event subscription authorization
- transcript/job persistence

Those belong to later event/projection/audit slices.
