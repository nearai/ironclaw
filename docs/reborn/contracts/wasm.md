# Reborn WASM runtime contract

The Reborn WASM runtime executes sandboxed extension components through the canonical component-model ABI declared in `wit/tool.wit`.

## ABI

- Tool components implement world `near:agent/sandboxed-tool@0.3.0`.
- The host imports are the `near:agent/host@0.3.0` interface:
  - `log`
  - `now-millis`
  - `workspace-read`
  - `http-request`
  - `tool-invoke`
  - `secret-exists`
- Tool components export `near:agent/tool@0.3.0`:
  - `description() -> string`
  - `schema() -> string`
  - `execute(request) -> response`

The abandoned JSON pointer/length ABI (`alloc`, `invoke_json`, `output_ptr`, `output_len`, and runtime-specific HTTP imports such as `http_request_utf8`) is not part of Reborn.

## Runtime invariants

- Compile once, instantiate a fresh component instance for every execution.
- Apply fuel, epoch-timeout, memory, table, and instance limits to every metadata and execution call.
- Allow component-model tools to instantiate multiple internal memories when needed, but enforce `WitToolLimits::memory_bytes` as an aggregate per-execution memory budget across all memories.
- Treat WIT metadata as the source of runtime compatibility: `description()` and `schema()` are called through generated Wasmtime component bindings.
- Keep V1 `src/tools/wasm/*` and `src/channels/wasm/*` as compatibility references only; Reborn is a separate binary path.

## Host capability seams

All host capabilities are injected through explicit Rust seams. The default host is fail-closed:

- HTTP egress returns an unavailable error unless a host implementation is injected.
- Workspace reads return `None` unless a workspace implementation is injected.
- Secret access is existence-only and returns `false` unless a secret implementation is injected.
- Nested tool invocation returns unavailable unless a tool implementation is injected.

Production HTTP must be wired to the shared Reborn runtime egress service tracked by #3085, not implemented directly inside `ironclaw_wasm`. The WASM runtime applies the WIT HTTP default timeout when `timeout-ms` is omitted, caps it to the remaining execution deadline, and reports a timeout if a host import returns after that deadline; injected synchronous host implementations must still honor the supplied timeout because they cannot be safely preempted mid-call.

## Network accounting

`ResourceUsage.network_egress_bytes` counts outbound request body bytes only. Response body limits and response scanning are separate host-egress responsibilities and must not be recorded as egress usage. If the host reports that a request was sent but later failed during response handling, the request body still counts as egress; fail-closed denials before send count zero. Execution failures preserve the usage/log snapshot collected before the failure so callers can reconcile sent egress even when the guest traps.
