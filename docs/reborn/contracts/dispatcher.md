# IronClaw Reborn dispatcher contract

Date: 2026-04-24
Status: V1 contract slice
Crate: `crates/ironclaw_dispatcher`

---

## 1. Purpose

`ironclaw_dispatcher` is the composition-only runtime dispatch layer for Reborn.

It connects already-validated extension capabilities to runtime lanes:

```text
ExtensionRegistry + RootFilesystem + ResourceGovernor + registered RuntimeAdapter backends
  -> RuntimeDispatcher::dispatch_json(...)
  -> selected adapter for RuntimeKind
  -> normalized CapabilityDispatchResult
```

The dispatcher does not discover extensions, parse manifests, implement policy, open files directly, resolve secrets, or execute product workflows. It wires service crates together and fails closed when a required lane or declaration is missing.

---

## 2. Inputs

The dispatcher receives an already-authorized `CapabilityDispatchRequest` from the neutral `ironclaw_host_api` dispatch port:

```rust
pub struct CapabilityDispatchRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub input: serde_json::Value,
}
```

The dispatcher can be constructed from borrowed service boundaries for request-scoped composition:

```rust
RuntimeDispatcher::new(&registry, &root_filesystem, &resource_governor)
    .with_runtime_adapter(RuntimeKind::Wasm, &wasm_adapter)
    .with_runtime_adapter(RuntimeKind::Script, &script_adapter)
    .with_runtime_adapter(RuntimeKind::Mcp, &mcp_adapter)
```

For detached background execution, it can also own shared service handles:

```rust
RuntimeDispatcher::from_arcs(registry, root_filesystem, resource_governor)
    .with_runtime_adapter_arc(RuntimeKind::Script, script_adapter)
```

The owned form keeps dispatcher composition-only while allowing the process-dispatch adapter to run capability-backed processes without leaking borrowed app state into a spawned task. `RuntimeDispatcher` implements the host API `CapabilityDispatcher` trait, but `ironclaw_capabilities` no longer depends on this concrete crate.

`ExtensionRegistry` remains the authority for what can run. Runtime adapter owners remain the authority for how a lane runs. The concrete WASM, Script, and MCP adapters now live in `ironclaw_host_runtime`, so `ironclaw_dispatcher` no longer has normal dependencies on `ironclaw_wasm`, `ironclaw_scripts`, or `ironclaw_mcp`.

---

## 3. Dispatch algorithm

V1 `dispatch_json` performs only routing and consistency checks:

```text
1. lookup capability in ExtensionRegistry
2. lookup provider package in ExtensionRegistry
3. verify descriptor.runtime == package.manifest.runtime_kind()
4. select the registered `RuntimeAdapter` for `RuntimeKind`
5. call the configured adapter for that lane
6. return a normalized host API dispatch result or host-safe failure kind
```

`RuntimeAdapter` is the open extension seam:

```rust
#[async_trait]
pub trait RuntimeAdapter<F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}
```

Each runtime adapter owns its local reserve/prepare/invoke/reconcile/release lifecycle. The dispatcher does not duplicate the resource-governor protocol and does not import concrete runtime crates.

---

## 4. Runtime lane status

V1 routes any `RuntimeKind` through a registered adapter:

| Runtime kind | Dispatch behavior |
| --- | --- |
| `Wasm` | Executes through a configured WASM adapter, usually composed by `ironclaw_host_runtime` |
| `Script` | Executes through a configured Script adapter, usually composed by `ironclaw_host_runtime` |
| `Mcp` | Executes through a configured MCP adapter, usually composed by `ironclaw_host_runtime` |
| `FirstParty` | Recognized as a host-service lane but not executed by default in this dispatcher slice |
| `System` | Recognized as a system-service lane but not executed by default in this dispatcher slice |

If a WASM, Script, or MCP runtime kind has no adapter configured, dispatch returns `MissingRuntimeBackend` before reserving resources. First-party/system lanes fail as `UnsupportedRuntime` unless a later host-service slice deliberately wires a scoped adapter.

---

## 5. Fail-closed rules

The dispatcher fails before execution when:

- capability ID is not registered
- provider package is not registered
- capability descriptor runtime does not match package manifest runtime
- selected runtime adapter is missing
- selected runtime adapter returns a typed dispatch failure

These failures must not reserve resources or perform external effects. Event sink failures are best-effort observability failures and must not turn a preflight or post-execution dispatch outcome into a different workflow result.

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
- MCP protocol handshake/lifecycle management beyond a registered adapter contract
- host service dispatch for first-party/system capabilities
- filesystem mount selection
- network or secret injection
- background `spawn` / process lifecycle
- agent-loop behavior

Those belong in dedicated service crates or later narrow dispatcher composition slices.

---

## 8. Contract tests

The crate test suite covers:

- WASM capability dispatch through a registered adapter
- unknown capability failure before resource reservation
- descriptor/package runtime mismatch failure before execution
- Script capability dispatch through a registered adapter
- MCP capability dispatch through a registered adapter
- first-party and system lanes recognized but not executed by default
- missing WASM, Script, or MCP adapter failure before resource reservation
- event sink failures ignored on both success and failure paths
- runtime failure details redacted to host-safe dispatch failure kinds

These tests are intentionally caller-level: they drive `RuntimeDispatcher::dispatch_json`, not only helper functions.
