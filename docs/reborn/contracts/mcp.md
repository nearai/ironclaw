# IronClaw Reborn MCP adapter contract

**Date:** 2026-06-02
**Status:** Hosted HTTP/SSE discovery slice
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
    async fn discover_tools(&self, request: McpClientRequest) -> Result<McpToolDiscoveryOutput, String>;
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
- HTTP/SSE credential planning is host-owned. The MCP client plans the real
  `tools/call` / `tools/list` JSON-RPC body once before the handshake, rejects direct
  `SecretStoreLease` sources before any transport request, and threads the
  approved staged plan into the eventual `tools/call` / `tools/list` send.
- Planner-visible headers exclude the dynamic MCP session header. The protocol
  client appends `Mcp-Session-Id` after planning when the server establishes a
  session.
- Hosted providers may require authentication on the whole JSON-RPC session,
  including `initialize`, `notifications/initialized`, `tools/list`, and
  `tools/call`. Staged credentials for MCP runtime egress therefore remain
  reusable for the scoped capability invocation and are discarded by the normal
  capability completion/abort cleanup.
- MCP protocol code consumes `RuntimeCredentialInjection` plans only; product
  auth account selection belongs to composition, not to the MCP crate.

## 7. Hosted schema discovery

`McpClient::discover_tools(...)` performs the same hosted HTTP/SSE handshake as
`call_tool`, then sends `tools/list` through the host-mediated egress boundary.
The protocol client parses the server result into bounded `McpDiscoveredTool`
records:

- tool names must be directly publishable as Reborn capability suffixes
  (lowercase ASCII name segments separated by dots); unsupported names are
  rejected rather than normalized so discovery cannot create ambiguous or
  colliding capability IDs
- descriptions are bounded and control-character-free
  except for normal formatting whitespace (`\n`, `\r`, `\t`)
- `inputSchema` must be an object-shaped JSON schema
- MCP annotations are parsed as behavior hints. `destructiveHint` and
  `sideEffectsHint` mark the discovered capability as `external_write`;
  `readOnlyHint` suppresses that effect when no stronger write hint is present.
  If annotations are absent and the bundled provider manifest declares any
  write-capable MCP tool, discovery keeps `external_write` as a conservative
  provider-level over-approximation.
- direct `SecretStoreLease` credential sources fail before the handshake
- staged product-auth credentials are allowed for `tools/list` when the host
  planner supplies them

Extension activation must choose an explicit activation mode. Static activation
publishes the bundled package; hosted MCP discovery activation performs
discovery before lifecycle state is committed. The discovery call runs outside
the lifecycle operation lock, then activation reacquires the lock and verifies
the installed package did not change before publishing. Successful discovery
replaces the provider package's capability declarations in the active
`SharedExtensionRegistry`; failed discovery fails activation instead of
publishing stale bundled schema guesses.

Discovered packages are built through the extension package constructor for
host-bundled inline dynamic schemas. The published descriptors carry inline
input schemas, while all non-schema descriptor fields must still match the
manifest projection exactly. This avoids fake schema files for hosted MCP
discovery without weakening ordinary extension descriptor consistency.

Discovery must be invoked with a real caller/run scope and host-staged
obligations. Reborn startup must not create an ambient scope, bypass the
network-policy store, or probe hosted MCP servers directly.

---

## 8. Non-goals

This slice does not implement:

- long-lived MCP server lifecycle management
- OAuth/auth flows for MCP servers
- raw secret injection
- broad network access
- filesystem mounts for MCP servers
- hosted sandbox backend selection
- projection/read-model generation
- conversation or agent-loop behavior

Those belong to later adapter, auth, network, process, and run-state slices.
