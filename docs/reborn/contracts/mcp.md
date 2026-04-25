# IronClaw Reborn MCP adapter contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_mcp`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/extensions.md`, `docs/reborn/contracts/resources.md`, `docs/reborn/contracts/dispatcher.md`

---

## 1. Purpose

`ironclaw_mcp` adapts manifest-declared MCP tools into IronClaw capabilities.

MCP is an integration lane, not an authority bypass:

```text
ExtensionPackage(runtime = mcp)
  -> McpRuntime validates manifest/capability metadata
  -> ResourceGovernor reserve(...)
  -> host-selected McpClient adapter call
  -> output limit enforcement
  -> ResourceGovernor reconcile(...) / release(...)
```

The crate does not discover extensions, grant secrets, open host paths, perform approval decisions, or expose unmediated network/process authority to models or MCP servers.

---

## 2. Runtime contract

The host configures:

```rust
McpRuntime::new(McpRuntimeConfig, impl McpClient)
```

The runtime accepts:

```rust
pub struct McpExecutionRequest<'a> {
    pub package: &'a ExtensionPackage,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub invocation: McpInvocation,
}
```

A successful execution returns the normalized lane result:

```rust
pub struct McpExecutionResult {
    pub result: McpCapabilityResult,
    pub receipt: ResourceReceipt,
}
```

The dispatcher then maps this into `CapabilityDispatchResult` with `runtime = RuntimeKind::Mcp`.

---

## 3. Host-selected MCP client

`McpClient` is the only adapter interface in this slice:

```rust
#[async_trait]
pub trait McpClient: Send + Sync {
    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String>;
}
```

`McpClientRequest` contains manifest-derived MCP metadata:

```rust
pub struct McpClientRequest {
    pub provider: ExtensionId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub input: serde_json::Value,
}
```

Important boundaries:

- command, args, URL, and transport come from the validated manifest, not model text
- raw host paths are not included
- raw secrets are not included
- MCP server network/process behavior remains host-adapter policy, not dispatcher policy
- stdio MCP usage is accounted as at least one process in V1

---

## 4. Resource lifecycle

`McpRuntime::execute_extension_json(...)` owns the MCP lane resource lifecycle:

```text
validate package/capability/runtime
  -> reserve(scope, estimate)
  -> client.call_tool(...)
  -> enforce output limit
  -> reconcile(reservation_id, actual_usage)
```

Failure cleanup:

```text
validation fails before reserve -> no reservation
reserve fails -> no client call
client failure -> release reservation
output limit failure -> release reservation
success -> reconcile reservation
```

The runtime computes serialized JSON output bytes and reconciles at least that amount. Stdio MCP transport records at least one process in actual usage.

---

## 5. Dispatcher

`RuntimeDispatcher` now supports:

```rust
RuntimeDispatcher::new(&registry, &filesystem, &governor)
    .with_mcp_runtime(&mcp_runtime)
```

For `RuntimeKind::Mcp`, dispatch emits the same runtime events as other lanes:

```text
dispatch_requested
runtime_selected
dispatch_succeeded / dispatch_failed
```

If no MCP runtime is configured, dispatch returns `MissingRuntimeBackend { runtime: RuntimeKind::Mcp }` before reserving resources.

---

## 6. V1 supported transport posture

The runtime recognizes manifest-declared `stdio`, `http`, and `sse` transport strings and passes them to the host-selected adapter. It does not implement raw protocol clients in the dispatcher.

Transport-specific policy is a host adapter responsibility:

- stdio process spawning should reuse the mediated process/sandbox substrate where appropriate
- HTTP/SSE transport must go through host network policy, not ambient network access
- secret injection must be explicit and audited in a later auth/secrets slice

---

## 7. Non-goals

This slice does not implement:

- MCP protocol handshakes or schema discovery
- long-lived MCP server lifecycle management
- OAuth/auth flows for MCP servers
- raw secret injection
- broad network access
- filesystem mounts for MCP servers
- hosted sandbox backend selection
- projection/read-model generation
- conversation or agent-loop behavior

Those belong to later adapter, auth, network, process, and run-state slices.
