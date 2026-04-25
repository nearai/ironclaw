# IronClaw Reborn kernel dispatch contract

Date: 2026-04-24
Status: V1 contract slice
Crate: `crates/ironclaw_kernel`

---

## 1. Purpose

`ironclaw_kernel` is the first composition-only dispatch layer for Reborn.

It connects already-validated extension capabilities to runtime lanes:

```text
ExtensionRegistry + RootFilesystem + ResourceGovernor + runtime backends
  -> RuntimeDispatcher::dispatch_json(...)
  -> selected runtime lane
  -> normalized CapabilityDispatchResult
```

The kernel does not discover extensions, parse manifests, implement policy, open files directly, resolve secrets, or execute product workflows. It wires service crates together and fails closed when a required lane or declaration is missing.

---

## 2. Inputs

The dispatcher receives a `CapabilityDispatchRequest`:

```rust
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub input: serde_json::Value,
}
```

The dispatcher is constructed from references to service boundaries:

```rust
RuntimeDispatcher::new(&registry, &root_filesystem, &resource_governor)
    .with_wasm_runtime(&wasm_runtime)
    .with_script_runtime(&script_runtime)
    .with_mcp_runtime(&mcp_runtime)
```

`ExtensionRegistry` remains the authority for what can run. Runtime crates remain the authority for how a lane runs.

---

## 3. Dispatch algorithm

V1 `dispatch_json` performs only routing and consistency checks:

```text
1. lookup capability in ExtensionRegistry
2. lookup provider package in ExtensionRegistry
3. verify descriptor.runtime == package.manifest.runtime_kind()
4. select runtime lane from RuntimeKind
5. call the configured backend for that lane
6. return normalized result or typed failure
```

For `RuntimeKind::Wasm`, the dispatcher calls:

```text
ironclaw_wasm::WasmRuntime::execute_extension_json(...)
```

For `RuntimeKind::Script`, the dispatcher calls:

```text
ironclaw_scripts::ScriptExecutor::execute_extension_json(...)
```

For `RuntimeKind::Mcp`, the dispatcher calls:

```text
ironclaw_mcp::McpExecutor::execute_extension_json(...)
```

Each runtime lane still owns its local reserve/prepare/invoke/reconcile/release lifecycle. The dispatcher does not duplicate the resource-governor protocol.

---

## 4. Runtime lane status

V1 routes these runtime kinds explicitly:

| Runtime kind | Dispatch behavior |
| --- | --- |
| `Wasm` | Executes through configured `WasmRuntime` |
| `Script` | Executes through configured `ScriptExecutor` |
| `Mcp` | Executes through configured `McpExecutor` adapter |
| `FirstParty` | Recognized, returns `UnsupportedRuntime` until host service adapters land |
| `System` | Recognized, returns `UnsupportedRuntime` until system service adapters land |

If the selected WASM, Script, or MCP runtime is not configured, dispatch returns `MissingRuntimeBackend` before reserving resources.

---

## 5. Fail-closed rules

The dispatcher fails before execution when:

- capability ID is not registered
- provider package is not registered
- capability descriptor runtime does not match package manifest runtime
- selected runtime backend is missing
- selected runtime lane is recognized but not implemented yet

These failures must not reserve resources or perform external effects.

---

## 6. Result shape

A successful dispatch returns a normalized result:

```rust
pub struct CapabilityDispatchResult {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub output: serde_json::Value,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
}
```

The shape intentionally exposes common host-level facts and avoids leaking WASM-specific internals as the generic contract.

---

## 7. Non-goals

This PR does not add:

- authorization/grant evaluation
- approval prompts
- full audit/event projection persistence
- script filesystem mounts, artifact export, network access, or secret injection
- MCP protocol handshake/lifecycle management beyond the adapter contract
- host service dispatch for first-party/system capabilities
- filesystem mount selection
- network or secret injection
- background `spawn` / process lifecycle
- agent-loop behavior

Those belong in dedicated service crates or later narrow kernel composition slices.

---

## 8. Contract tests

The crate test suite covers:

- WASM capability dispatch through the real WASM executor
- unknown capability failure before resource reservation
- descriptor/package runtime mismatch failure before execution
- Script capability dispatch through a configured script executor
- MCP capability dispatch through a configured MCP executor
- first-party and system lanes recognized but not executed
- missing WASM, Script, or MCP backend failure before resource reservation

These tests are intentionally caller-level: they drive `RuntimeDispatcher::dispatch_json`, not only helper functions.
