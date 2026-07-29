# IronClaw Reborn live vertical slice

**Date:** 2026-04-25
**Status:** Runnable V1 demo
**Crates:** `ironclaw_filesystem`, `ironclaw_extensions`, `ironclaw_resources`, `ironclaw_events`, `ironclaw_dispatcher`, `ironclaw_host_runtime`, `ironclaw_scripts`

---

## 1. Purpose

This slice proves the first Reborn host path is runnable:

```text
DiskFilesystem mounted at /system/extensions
-> ExtensionDiscovery reads manifests
-> ExtensionRegistry registers capabilities
-> RuntimeDispatcher receives sealed `Authorized` witnesses
-> RuntimeDispatcher resolves prebound adapters and validates the sealed `RuntimeLane`
-> dispatcher test bindings execute JSON echo capabilities
-> HostRuntimeServices examples wrap real ScriptRuntime backends for end-to-end capability/process demos
-> InMemoryResourceGovernor reserves and reconciles all invocations
-> JsonlEventSink records requested/selected/succeeded events under tenant/user/agent-scoped /engine event paths
-> JSON outputs are returned through one dispatch path
```

It is intentionally not a product agent loop, gateway, TUI, secret flow, or full event bus. The current event slice is dispatcher-level observability only, and the MCP slice is an adapter contract rather than a full MCP protocol/server lifecycle implementation.

---

## 2. Run it

```bash
cargo test -p ironclaw_dispatcher --test dispatch_contract --test event_dispatch_contract
```

Expected output shape:

```text
dispatcher contract tests pass, including dispatch_requested ->
runtime_selected -> dispatch_succeeded event ordering for resolved bindings.
```

The dispatcher contract tests use in-crate echo bindings so `ironclaw_dispatcher`
can demonstrate routing, resource lifecycle, and event emission without
depending on concrete WASM, Script, or MCP runtime crates. Real runtime wiring
now lives in `ironclaw_host_runtime`, which adapts configured runtimes into
dispatcher bindings and then drives capability/process workflows without Docker
by default.

---

## 3. What this validates

Implementation evidence: `crates/ironclaw_dispatcher/src/lib.rs` implements the
sealed `RuntimeDispatcher::dispatch_json(Authorized)` path, and
`crates/ironclaw_dispatcher/tests/dispatch_contract.rs` plus
`crates/ironclaw_dispatcher/tests/event_dispatch_contract.rs` validate the
dispatcher contracts that replaced the retired vertical-slice test.

The dispatcher contract tests validate:

- extension manifests are read from `DiskFilesystem` via `/system/extensions`
- extension discovery returns WASM, Script, and MCP packages
- dispatcher receives already-authorized sealed `Authorized` witnesses, resolves
  prebound adapters, and validates their sealed `RuntimeLane`
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
