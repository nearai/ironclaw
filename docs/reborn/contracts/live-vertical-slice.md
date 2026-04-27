# IronClaw Reborn live vertical slice

**Date:** 2026-04-25
**Status:** Runnable V1 demo
**Crates:** `ironclaw_filesystem`, `ironclaw_extensions`, `ironclaw_resources`, `ironclaw_events`, `ironclaw_dispatcher`, `ironclaw_host_runtime`, `ironclaw_scripts`

---

## 1. Purpose

This slice proves the first Reborn host path is runnable:

```text
LocalFilesystem mounted at /system/extensions
-> ExtensionDiscovery reads manifests
-> ExtensionRegistry registers capabilities
-> RuntimeDispatcher receives already-authorized dispatch requests
-> RuntimeDispatcher routes dispatch by RuntimeKind through registered adapters
-> dispatcher example adapters execute JSON echo capabilities
-> HostRuntimeServices examples wrap real ScriptRuntime backends for end-to-end capability/process demos
-> InMemoryResourceGovernor reserves and reconciles all invocations
-> JsonlEventSink records requested/selected/succeeded events under tenant/user/agent-scoped /engine event paths
-> JSON outputs are returned through one dispatch path
```

It is intentionally not a product agent loop, gateway, TUI, secret flow, or full event bus. The current event slice is dispatcher-level observability only, and the MCP slice is an adapter contract rather than a full MCP protocol/server lifecycle implementation.

---

## 2. Run it

```bash
cargo run -p ironclaw_dispatcher --example reborn_echo
```

Expected output shape:

```text
reborn_dispatcher_adapter_slice=ok
discovered_extensions=3
dispatch=echo-wasm.say runtime=wasm output={"message":"hello wasm"} reservation_status=Reconciled
dispatch=echo-script.say runtime=script output={"message":"hello script"} reservation_status=Reconciled
dispatch=echo-mcp.say runtime=mcp output={"message":"hello mcp"} reservation_status=Reconciled
durable_event_path=VirtualPath("/engine/tenants/tenant1/users/user1/agents/_none/events/runtime/reborn-demo.jsonl")
events=9
event[0]=dispatch_requested capability=echo-wasm.say runtime=none error=none
event[1]=runtime_selected capability=echo-wasm.say runtime=wasm error=none
event[2]=dispatch_succeeded capability=echo-wasm.say runtime=wasm error=none
event[3]=dispatch_requested capability=echo-script.say runtime=none error=none
event[4]=runtime_selected capability=echo-script.say runtime=script error=none
event[5]=dispatch_succeeded capability=echo-script.say runtime=script error=none
event[6]=dispatch_requested capability=echo-mcp.say runtime=none error=none
event[7]=runtime_selected capability=echo-mcp.say runtime=mcp error=none
event[8]=dispatch_succeeded capability=echo-mcp.say runtime=mcp error=none
```

The default dispatcher example uses in-crate echo adapters so `ironclaw_dispatcher` can demonstrate routing, resource lifecycle, and event emission without depending on concrete WASM, Script, or MCP runtime crates. Real runtime wiring now lives in `ironclaw_host_runtime`, whose examples use `HostRuntimeServices` to adapt configured runtimes into dispatcher adapters and then drive capability/process workflows without Docker by default.

---

## 3. What this validates

The integration test `crates/ironclaw_dispatcher/tests/vertical_slice_contract.rs` validates:

- extension manifests are read from `LocalFilesystem` via `/system/extensions`
- extension discovery returns WASM, Script, and MCP packages
- dispatcher crate tests exercise already-authorized `CapabilityDispatchRequest` values directly
- higher-level caller workflow stays out of dispatcher crate dev surfaces
- WASM dispatch goes through `RuntimeDispatcher` and a registered runtime adapter
- Script dispatch goes through `RuntimeDispatcher` and a registered runtime adapter
- MCP dispatch goes through `RuntimeDispatcher` and a registered runtime adapter
- all invocations reserve and reconcile resource usage
- all lanes emit dispatch requested/runtime selected/dispatch succeeded events
- event history is durably written through `RootFilesystem` at the scoped runtime event path
- all lanes return JSON output through the same normalized dispatch result type

---

## 4. Non-goals

This slice does not add:

- full realtime event bus fanout/reconnect
- durable transcript/job state
- approval resolution/resume
- scoped script filesystem mounts
- artifact export
- secret injection
- network access for scripts or MCP servers
- full MCP protocol handshake/server lifecycle
- conversation or agent-loop behavior

Those are follow-on slices once this dispatch path is stable.
