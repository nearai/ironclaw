# IronClaw Reborn live vertical slice

**Date:** 2026-04-25
**Status:** Runnable V1 demo
**Crates:** `ironclaw_filesystem`, `ironclaw_extensions`, `ironclaw_resources`, `ironclaw_wasm`, `ironclaw_scripts`, `ironclaw_events`, `ironclaw_kernel`

---

## 1. Purpose

This slice proves the first Reborn host path is runnable:

```text
LocalFilesystem mounted at /system/extensions
-> ExtensionDiscovery reads manifests
-> ExtensionRegistry registers capabilities
-> RuntimeDispatcher routes by RuntimeKind
-> WasmRuntime executes a WASM capability
-> ScriptRuntime executes a script capability
-> InMemoryResourceGovernor reserves and reconciles both invocations
-> JsonlEventSink records requested/selected/succeeded events under /engine/events
-> JSON outputs are returned through one dispatch path
```

It is intentionally not a product agent loop, gateway, TUI, MCP adapter, secret flow, or full event bus. The current event slice is dispatcher-level observability only.

---

## 2. Run it

```bash
cargo run -p ironclaw_kernel --example reborn_echo
```

Expected output shape:

```text
reborn_vertical_slice=ok
discovered_extensions=2
dispatch=echo-wasm.say runtime=wasm output={"message":"hello wasm"} reservation_status=Reconciled
dispatch=echo-script.say runtime=script script_backend=in_process_echo output={"message":"hello script"} reservation_status=Reconciled
events=6
durable_event_path=VirtualPath("/engine/events/reborn-demo.jsonl")
event[0]=dispatch_requested capability=echo-wasm.say runtime=none error=none
event[1]=runtime_selected capability=echo-wasm.say runtime=wasm error=none
event[2]=dispatch_succeeded capability=echo-wasm.say runtime=wasm error=none
event[3]=dispatch_requested capability=echo-script.say runtime=none error=none
event[4]=runtime_selected capability=echo-script.say runtime=script error=none
event[5]=dispatch_succeeded capability=echo-script.say runtime=script error=none
```

The default example uses an in-process echo script backend so the demo works without Docker installed. It still exercises the real `ScriptRuntime`, manifest-derived command metadata, `RuntimeDispatcher`, resource lifecycle, and event emission path.

---

## 3. Optional Docker backend

To exercise the V1 Docker script backend:

```bash
IRONCLAW_REBORN_DEMO_DOCKER=1 cargo run -p ironclaw_kernel --example reborn_echo
```

The script manifest declares:

```toml
[runtime]
kind = "script"
backend = "docker"
image = "alpine:latest"
command = "sh"
args = ["-c", "cat"]
```

`DockerScriptBackend` runs the command as:

```text
docker run --rm -i --network none alpine:latest sh -c cat
```

The example writes invocation JSON to stdin and expects JSON on stdout.

Docker availability, image presence, and local Docker permissions are intentionally environment-specific. The default non-Docker backend exists to keep the vertical slice runnable everywhere.

---

## 4. What this validates

The integration test `crates/ironclaw_kernel/tests/vertical_slice_contract.rs` validates:

- extension manifests are read from `LocalFilesystem` via `/system/extensions`
- extension discovery returns both WASM and Script packages
- WASM dispatch goes through `RuntimeDispatcher` and `WasmRuntime`
- Script dispatch goes through `RuntimeDispatcher` and `ScriptRuntime`
- both invocations reserve and reconcile resource usage
- both lanes emit dispatch requested/runtime selected/dispatch succeeded events
- event history is durably written through `RootFilesystem` at `/engine/events/reborn-demo.jsonl`
- both lanes return JSON output through the same normalized kernel result type

---

## 5. Non-goals

This slice does not add:

- full realtime event bus fanout/reconnect
- durable transcript/job state
- approval/auth gates
- scoped script filesystem mounts
- artifact export
- secret injection
- network access for scripts
- MCP adapter execution
- conversation or agent-loop behavior

Those are follow-on slices once this dispatch path is stable.
