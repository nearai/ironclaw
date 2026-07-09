# IronClaw Reborn MCP adapter contract

**Date:** 2026-07-09
**Status:** Hosted HTTP/SSE registration and discovery slice
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
    pub resource_reservation: Option<ResourceReservation>,
    pub invocation: McpInvocation,
}
```

A successful execution returns the normalized lane result:

```rust
pub struct McpExecutionResult {
    pub result: CapabilityHostResult,
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
    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, McpClientError>;
    async fn discover_tools(&self, request: McpClientRequest) -> Result<McpToolDiscoveryOutput, McpClientError>;
}
```

`McpClientRequest` contains manifest-derived MCP metadata:

```rust
pub struct McpClientRequest {
    pub provider: ExtensionId,
    pub authority: McpRequestAuthority,
    pub scope: ResourceScope,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub input: serde_json::Value,
    pub max_output_bytes: u64,
}

pub enum McpRequestAuthority {
    Capability(CapabilityId),
    ProviderDiscovery,
}
```

Important boundaries:

- command, args, URL, and transport come from the validated manifest, not model text
- normal dispatch carries a real `Capability` id; zero-capability registered
  discovery carries `ProviderDiscovery`, so no synthetic discovery capability
  can become a descriptor, staged obligation, or approval key
- raw host paths are not included
- raw secrets are not included
- MCP server network/process behavior remains host-adapter policy, not dispatcher policy
- external stdio MCP transport remains fail-closed until process-level egress
  controls are available

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

## 6. Supported transport posture

The runtime executes manifest-declared `http` and `sse` transports only through
the host-selected adapter. It recognizes but rejects external `stdio` transport
until process-level egress controls land. The dispatcher does not implement raw
protocol clients.

Transport-specific policy is a host adapter responsibility:

- future stdio process spawning must reuse the mediated process/sandbox substrate
- HTTP/SSE transport must go through host network policy, not ambient network access
- `initialize`, `notifications/initialized`, `tools/list`, and `tools/call`
  all cross the host-owned `RuntimeHttpEgress`; MCP protocol code never creates
  a direct outbound HTTP client
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
- `ProviderDiscovery` is credential-free in this slice. Its planner receives
  the registered endpoint's locked network policy but cannot derive a
  capability-scoped credential injection.
- During capability-driven activation, `ProviderDiscovery` borrows the outer
  `builtin.extension_activate` identity only as the exact staged network-policy
  lookup key; it remains unavailable as a callable capability, descriptor,
  approval key, or credential authority. Product surfaces such as WebUI and CLI
  do not invent that staged identity: they bind the provider identity and
  sandbox trust explicitly through `HostRuntimeHttpEgressPort`, which stages
  and cleans the locked network policy around every transport request.

## 7. Hosted schema discovery

`McpClient::discover_tools(...)` performs the same hosted HTTP/SSE handshake as
`call_tool`, then sends `tools/list` through the host-mediated egress boundary.
The protocol client parses the server result into bounded indexed tool
candidates plus bounded structured rejections:

- tool names must be directly publishable as Reborn capability suffixes
  (lowercase ASCII name segments separated by dots). An unsupported name skips
  only that entry and returns its index plus the host-authored
  `unsupported_name` reason; the name and other raw server content are neither
  retained in the rejection nor logged. Other response-shape, description,
  schema, annotation, and size violations still fail the complete list closed
- descriptions are bounded and control-character-free
  except for normal formatting whitespace (`\n`, `\r`, `\t`)
- `inputSchema` must be an object-shaped JSON schema
- MCP annotations are parsed as behavior hints. `destructiveHint` and
  `sideEffectsHint` mark the discovered capability as `external_write`;
  `readOnlyHint` suppresses that effect when no stronger write hint is present.
  If annotations are absent and the bundled provider manifest declares any
  write-capable MCP tool, discovery keeps `external_write` as a conservative
  provider-level over-approximation.
- every capability discovered from a `UserRegistered` provider is forced to
  `ExternalWrite`, including tools that claim `readOnlyHint=true`; bundled
  providers retain the annotation/template behavior above
- direct `SecretStoreLease` credential sources fail before the handshake
- staged product-auth credentials are allowed for `tools/list` when the host
  planner supplies them

Extension activation must choose an explicit activation mode. Static activation
publishes an ordinary package; hosted MCP discovery activation performs
discovery before lifecycle state is committed. The discovery call runs outside
the lifecycle operation lock, then activation reacquires the lock and verifies
the installed package did not change before publishing. Successful discovery
replaces the provider package's capability declarations in the active
`SharedExtensionRegistry`. Bundled providers preserve their transient fallback
to the bundled declarations. A registered provider has no static declarations,
so transient, empty, or all-invalid discovery fails activation rather than
publishing its empty base descriptor.

Discovered packages are built through the extension package constructor for
inline dynamic schemas from `HostBundled` and `UserRegistered` sources only.
The published descriptors carry inline input schemas, while all non-schema
descriptor fields must still match the manifest projection exactly. A
zero-capability registered descriptor receives an in-memory template requiring
only `host.runtime.http_egress`; the persisted descriptor remains
zero-capability.

Before registered tools are published, composition scans both the description
and serialized input schema with the shared injection scanner. High or Critical
findings quarantine that capability without suppressing safe siblings. A
quarantined capability is owner-visible for operator status but uses a fixed
host-authored description, an inert object schema, `HostInternal` visibility,
and `Deny` permission. Raw server description/schema text is not retained in
the quarantined descriptor or exposed to the model. Lower-severity findings are
audit-only. Bundled provider metadata is not reclassified by this registered-
content safety rule.

Discovery must be invoked with a real caller/run scope and host-staged
obligations. Reborn startup must not create an ambient scope, bypass the
network-policy store, or probe hosted MCP servers directly.

## 8. Per-user registration, restart, and revocation

Authenticated WebChat v2 callers manage hosted HTTPS MCP descriptors through:

```text
POST /api/webchat/v2/extensions/register
  body: { "name": string, "url": string }
  response: { "package_ref": ..., "extension_id": string }

POST /api/webchat/v2/extensions/{package_id}/unregister
```

Registration validates an HTTPS public target, mints an owner-and-normalized-
URL-bound extension id, persists a zero-capability `UserRegistered` descriptor,
and installs it for that owner. Registration does not probe authentication and
T3 supports `auth = none` only. Activation performs the live MCP handshake and
`tools/list` through `RuntimeHttpEgress`; registration itself does not persist
tool descriptions, schemas, annotations, or quarantine text.

Live discovered definitions are intentionally in-memory. On restart, an
enabled registered installation is changed to `Disabled`, its zero-capability
base descriptor is not published to the active registry, and no model tool is
disclosed. The owner must explicitly activate it again to rediscover the live
surface. This preserves the invariant that `Enabled` means a live package is
actually published.

The registered-extension store persists only the bounded, sorted current set
of discovered capability ids under the owner and extension identity, outside
the descriptor subtree. This ID-only cleanup inventory survives descriptor
deletion. Rediscovery revokes policies and clears permission overrides for ids
that disappeared before replacing the inventory. Unregister uses the inventory
to revoke exact tenant-user dispatch policies and clear exact per-tool
overrides, unpublishes/removes the extension, deletes the owner descriptor, and
deletes cleanup intent only after the preceding steps succeed. Missing records
are successful so retries remain safe; unrelated capability authority is not
changed. Cleanup-intent deletion is after the removal commit point: if that
final storage delete fails, the host logs the failure and still reports the
removal as committed rather than rolling back a same-name replacement. The
bounded stale ID-only record is safe and is replaced or retried by a later
same-id activation/unregister.

Future API-key or OAuth registration must store raw credential material only
through the encrypted `ironclaw_secrets::SecretStore`. Manifests and account
records may carry only `SecretHandle`s. Plaintext may be leased and injected
only inside the authorized `RuntimeHttpEgress` request window, with matching
redaction values, and must never enter MCP manifests, discovered descriptors,
model requests, transcripts, tool results, or logs.

---

## 9. Non-goals

This slice does not implement:

- long-lived MCP server lifecycle management
- API-key/OAuth registration flows for MCP servers in T3
- raw secret injection
- broad network access
- filesystem mounts for MCP servers
- hosted sandbox backend selection
- projection/read-model generation
- conversation or agent-loop behavior

Those belong to later adapter, auth, network, process, and run-state slices.
