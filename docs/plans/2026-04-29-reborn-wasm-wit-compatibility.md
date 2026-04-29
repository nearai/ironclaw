# Reborn WIT-Compatible WASM Runtime Plan

## Context

PR #3028 currently lands a hardened Reborn WASM runtime substrate, but its first implementation uses a custom JSON pointer/length ABI. That is not compatible with the existing production WASM tools and channels, which are built as Wasmtime component-model modules against the repository's canonical WIT files:

- `wit/tool.wit`
- `wit/channel.wit`

Reborn is intended to ship as a separate binary, so this plan does **not** refactor or migrate the existing V1 code in `src/tools/wasm/` or `src/channels/wasm/`. Those modules remain a compatibility reference/oracle only. The Reborn binary should get a new WIT-compatible runtime path that can load existing WASM tools/channels unchanged at the external contract level: WIT interface + sidecar capability schema + expected callback behavior.

The user direction for this plan is explicit:

- The JSON ABI can be thrown away.
- The Reborn runtime should use WIT/component model.
- Existing WASM tools and channels should be compatibility targets.
- All outbound HTTP in Reborn WASM must go through the kernel network crate.
- All secret lookup/lease/redaction in Reborn WASM must go through the kernel secrets crate.
- Do not touch current V1 WASM runtime code because Reborn is a different binary.

## Locked design decisions

1. **WIT is the production ABI**: `crates/ironclaw_wasm` should use Wasmtime component model and generated bindings from `wit/tool.wit` / `wit/channel.wit`. The custom JSON ABI from PR #3028 should not become a public compatibility path.
2. **V1 is not modified**: `src/tools/wasm/*` and `src/channels/wasm/*` remain untouched. They are only used as behavioral references for Reborn compatibility tests and sidecar mapping.
3. **Kernel network owns HTTP**: Reborn runtime code must not build direct `reqwest` clients, perform ad-hoc DNS resolution, or implement local SSRF checks. `ironclaw_network` owns outbound HTTP policy and execution.
4. **Shared runtime egress, not WASM-only**: The HTTP gateway is tracked by #3085 and is shared by WASM, Script, and host-mediated MCP runtime lanes. WASM keeps only a thin WIT adapter over that shared service.
5. **Kernel secrets own secret material**: Reborn runtime code must not access app-level V1 secret stores directly. `ironclaw_secrets` owns secret metadata, one-shot leases, and redaction handles.
6. **The guest should not see raw secrets**: Existing host-controlled credential injection behavior should be preserved. Guests can check `secret-exists` where allowed, but raw secret values are leased and injected by the host.
7. **Resource egress invariant**: `ResourceUsage.network_egress_bytes` counts outbound request bytes only. Response body limits and response byte accounting are separate.
8. **Fresh instance isolation**: Match V1's safety model: tools get a fresh instance per execution; channels get a fresh instance per callback, with host-managed state persisted between callbacks.
9. **Compatibility starts with tools**: WIT tools are smaller and should land before WIT channels. Channel routing/polling/webhook integration is a separate slice.

## Target architecture

```text
ironclaw-reborn binary
  └── Reborn host/runtime composition
        ├── ironclaw_dispatcher
        ├── ironclaw_wasm          # WIT tool/channel runtime
        ├── ironclaw_secrets       # secret leases, metadata, redaction
        ├── ironclaw_network       # all outbound HTTP transport/policy
        ├── runtime HTTP egress    # shared WASM/Script/MCP gateway (#3085)
        ├── ironclaw_filesystem    # workspace/filesystem grants
        └── ironclaw_resources     # reserve/reconcile accounting
```

WIT host imports flow through Reborn host services:

```text
Existing WASM component compiled against wit/tool.wit or wit/channel.wit
        │
        ▼
wasmtime::component generated bindings
        │
        ▼
ironclaw_wasm WIT adapter
        │
        ├── log/time collector
        ├── workspace/filesystem bridge
        ├── resource governor
        ├── secret bridge via ironclaw_secrets
        └── HTTP adapter -> shared runtime egress (#3085) -> ironclaw_network
```

## Non-goals

- Do not change `src/tools/wasm/wrapper.rs`.
- Do not change `src/channels/wasm/wrapper.rs`.
- Do not migrate current V1 WASM HTTP to kernel network in this plan.
- Do not fork the WIT files unless a future compatibility versioning decision explicitly requires it.
- Do not expose the JSON ABI from PR #3028 as the stable production Reborn WASM ABI.
- Do not implement full channel product routing in the same slice as WIT tool execution.

## Phase 1: Land/base on kernel secrets and network

Base the WIT rewrite on the Reborn kernel substrates:

- `crates/ironclaw_secrets`
- `crates/ironclaw_network`

If the secrets/network substrate PR is not merged yet, either land it first or stack the WIT branch on top of it. Reborn WASM should not grow temporary app-level secret or network integrations that will be removed later.

Acceptance criteria:

- Reborn WASM can depend on the kernel contracts without depending on V1 app modules.
- Boundary tests confirm dependency direction remains from WASM/host composition into kernel crates, not the reverse.

## Phase 2: Extend `ironclaw_network` from policy boundary to egress service

`ironclaw_network` should own all outbound HTTP for Reborn. Add a production egress trait and implementation.

Suggested shape:

```rust
#[async_trait]
pub trait NetworkEgress: Send + Sync {
    async fn execute(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError>;
}

pub struct NetworkRequest {
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub policy: NetworkPolicy,
    pub response_body_limit: Option<u64>,
    pub timeout: Duration,
}

pub struct NetworkResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub usage: NetworkUsage,
}

pub struct NetworkUsage {
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub resolved_ip: Option<IpAddr>,
}
```

`ironclaw_network` owns:

- URL parsing and normalization.
- Method validation.
- Target allowlist checks.
- DNS resolution.
- Private/loopback/link-local/documentation IP denial.
- Redirect policy.
- DNS rebinding protection / pinned resolved IP connection behavior.
- Request body byte limit.
- Streaming response body byte limit.
- Sanitized network errors.
- Request/response byte accounting.

Tests should prove:

- Unlisted hosts are denied before client dispatch.
- Private literal IPs are denied before client dispatch.
- Hostnames resolving to private IPs are denied before client dispatch.
- Redirects cannot bypass the original network policy.
- Response bodies are capped while streaming, not only after full allocation.
- `NetworkUsage.request_bytes` is used for `ResourceUsage.network_egress_bytes`.
- Response bytes are tracked separately and never counted as egress.

## Phase 3: Extend `ironclaw_secrets` for leases and redaction

Reborn WASM needs host-side secret access for credential injection and metadata checks. Add or expose a lease-oriented surface in `ironclaw_secrets`.

Suggested shape:

```rust
pub struct SecretLease { /* no Debug with raw value */ }

#[async_trait]
pub trait SecretLeaseStore: Send + Sync {
    async fn lease_secret(
        &self,
        scope: SecretScope,
        name: SecretName,
    ) -> Result<SecretLease, SecretError>;
}
```

Required behavior:

- Secret values are not included in `Debug`, errors, traces, or snapshots.
- Leases are scoped to one host operation.
- Missing required credentials fail closed.
- Missing optional credentials are skipped.
- Redaction can scrub leased values from network errors, guest-visible errors, logs, and traces.
- `secret-exists` checks metadata/policy without exposing values.

Credential injection remains host-controlled:

```text
credential mapping + secret lease
  -> inject into URL/header/query/body
  -> call ironclaw_network
  -> redact any leased values
  -> drop lease
```

## Phase 4: Add shared Reborn runtime HTTP egress (#3085)

Add a Reborn-only shared runtime egress service that composes `ironclaw_secrets` + `ironclaw_network`. This is not WASM-only and does not live in V1 code.

The shared service is used by all Reborn capability runtime lanes that need host-mediated HTTP:

```text
WIT http-request import -> WIT adapter -> RuntimeHttpEgress -> ironclaw_secrets + ironclaw_network
Script host HTTP API    -> Script adapter -> RuntimeHttpEgress -> ironclaw_secrets + ironclaw_network
MCP host-mediated HTTP  -> MCP adapter -> RuntimeHttpEgress -> ironclaw_secrets + ironclaw_network
```

The runtime-specific adapters translate runtime-native request/response shapes into the shared request type. They do not own network policy, DNS, redirects, SSRF checks, credential leasing, or actual HTTP transport.

Possible locations:

- `crates/ironclaw_host_runtime/src/http_egress.rs` for the production shared service if host-runtime composition exists by then.
- `crates/ironclaw_wasm/src/host/http.rs` only for the thin WIT adapter/types needed by generated WIT host imports.
- Script/MCP runtime crates should add similarly thin adapters when they expose host-mediated HTTP.

The shared service owns the translation from runtime host calls to kernel network/secrets calls:

```text
runtime HTTP call
  -> validate capability/network policy
  -> lease/inject secrets through ironclaw_secrets
  -> call ironclaw_network::NetworkEgress
  -> redact errors/logs/traces
  -> return runtime-compatible response
```

Suggested shared request/response types:

```rust
pub struct RuntimeHttpEgressRequest {
    pub runtime: RuntimeKind,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub credential_policy: RuntimeCredentialPolicy,
    pub network_policy: NetworkPolicy,
    pub response_body_limit: Option<u64>,
}

pub struct RuntimeHttpEgressResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub request_bytes: u64,
    pub response_bytes: u64,
}
```

MCP and arbitrary-process script modes need an explicit bypass-prevention story. If a runtime can open sockets directly, host-mediated egress is not sufficient by itself; that runtime must use sandbox/proxy/network-namespace controls or fail closed until process-level enforcement exists.

Ratchet tests should fail if Reborn runtime code directly uses:

- `reqwest::Client`
- `reqwest::ClientBuilder`
- `ToSocketAddrs` / `to_socket_addrs`
- V1 `ssrf_safe_client_builder*`
- V1 `validate_and_resolve_http_target`
- V1 `reject_private_ip`

Outside of tests and `crates/ironclaw_network`, direct outbound HTTP should be forbidden.

## Phase 5: Rewrite PR #3028 as a WIT-only tool runtime

Delete/replace the JSON ABI runtime pieces from `crates/ironclaw_wasm`:

- `invoke_json`
- `invoke_i32`
- export-based `WasmModuleSpec`
- `alloc` / `output_ptr` / `output_len` ABI requirements
- `http_request_utf8`
- JSON ABI tests
- local network policy wrapper if it duplicates `ironclaw_network`

Keep and adapt the hardened runtime pieces:

- Wasmtime configuration.
- Fuel metering.
- Epoch interruption / wall-clock timeout.
- Memory limiter.
- Compiled component cache.
- Fresh instance per execution.
- Resource reserve/reconcile/release pattern.
- Fail-closed unsupported import posture.

Use Wasmtime component model:

```rust
wasmtime::component::bindgen!({
    path: "../../wit/tool.wit",
    world: "sandboxed-tool",
});
```

Add WASI p2 support with no ambient authority:

```rust
wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
let wasi = WasiCtxBuilder::new().build();
```

No ambient filesystem, env vars, process access, or network access.

Suggested structure:

```text
crates/ironclaw_wasm/src/
  lib.rs
  config.rs
  engine.rs
  limits.rs
  wit_tool.rs
  host/
    mod.rs
    http.rs
    secrets.rs
    workspace.rs
    logs.rs
```

## Phase 6: Implement WIT tool runtime compatibility

Add a WIT tool execution surface:

```rust
pub struct WitToolRuntime { /* shared Wasmtime engine/cache/config */ }
pub struct PreparedWitTool { /* compiled component + metadata + policy */ }
pub struct WitToolRequest { /* params, context, resource reservation/scope */ }
pub struct WitToolResult { /* output, logs, usage, fuel, receipt */ }
```

Support current `wit/tool.wit`:

- Instantiate component.
- Call `description()`.
- Call `schema()`.
- Call `execute(request)`.

Host imports map to Reborn services:

| WIT tool import | Reborn backing service |
| --- | --- |
| `log` | guest log collector / event sink |
| `now-millis` | clock |
| `workspace-read` | Reborn workspace/filesystem service |
| `http-request` | WIT adapter -> shared runtime HTTP egress (#3085) -> `ironclaw_network` |
| `secret-exists` | `ironclaw_secrets` metadata/policy check |
| `tool-invoke` | fail closed or preserve V1's current unsupported behavior initially |

Acceptance criteria:

- An existing WIT tool component loads unchanged.
- `description()` and `schema()` extraction work.
- `execute()` works with V1-compatible request/response types.
- Host imports fail closed without matching grants.
- HTTP always goes through the shared runtime HTTP egress service (#3085), with WASM using only a thin WIT adapter.
- Secrets always go through `ironclaw_secrets`.
- Fuel/memory/timeout/output protections still apply.

## Phase 7: Add WIT tool sidecar compatibility

Existing WASM tools have sidecar capability files. Reborn should accept the same sidecar shape without importing V1 app modules directly.

Preferred location:

```text
crates/ironclaw_wasm_manifest
```

Short-term acceptable location:

```text
crates/ironclaw_wasm/src/manifest.rs
```

Map current sidecar concepts into Reborn policies:

| Current tool sidecar field | Reborn mapping |
| --- | --- |
| HTTP allowlist | `ironclaw_network::NetworkPolicy.allowed_targets` |
| max request bytes | `NetworkPolicy.max_egress_bytes` |
| max response bytes | separate response body limit |
| credential mappings | `ironclaw_secrets` lease/injection plan |
| workspace read prefixes | Reborn workspace/filesystem grants |
| allowed secret names | secret metadata visibility policy |
| WIT version | Reborn WIT compatibility check |

Compatibility rule:

```text
Current *.capabilities.json files should not require edits to run under Reborn.
```

## Phase 8: Wire WIT tools into Reborn dispatch

Implement a runtime adapter for `RuntimeKind::Wasm`, likely in host-runtime composition rather than inside `ironclaw_wasm` itself.

The adapter converts:

```text
RuntimeAdapterRequest
  -> WitToolRequest
  -> WitToolResult
  -> RuntimeAdapterResult
```

Error mapping requirements:

- No raw secret values in errors/events.
- No sensitive network internals in durable events.
- Stable dispatcher-safe error kinds.
- Prepared resource reservations are reconciled or released exactly once.
- Failed release handling should not silently hide the original execution failure; prefer logging release failures while preserving the dispatch error when safe.

Acceptance criteria:

- `RuntimeDispatcher` can invoke `RuntimeKind::Wasm` through the WIT runtime adapter.
- Missing WASM backend still emits existing safe missing-backend behavior.
- WASM backend failures emit sanitized event kinds.

## Phase 9: Add Reborn WIT tool loader

Because Reborn is a separate binary, implement a Reborn-native loader rather than reusing V1 loader internals.

Initial supported locations can include:

- `~/.ironclaw/tools/*.wasm`
- `tools-src/*/target/wasm32-wasip2/release/*.wasm`
- database-backed installed tools later

Loader responsibilities:

- Read WASM component bytes.
- Read sidecar capabilities.
- Check WIT version compatibility.
- Prepare the WIT component.
- Register capability descriptors with Reborn registry/dispatcher.
- Attach kernel-backed host policies and credential mappings.

Acceptance criteria:

- At least one current tool fixture loads through the Reborn loader.
- Loader errors are actionable and do not leak secrets.
- Binary content hash is recorded for cache/storage identity.

## Phase 10: Implement WIT channel runtime

After WIT tools are working, add channel runtime compatibility as a separate slice.

Use:

```rust
wasmtime::component::bindgen!({
    path: "../../wit/channel.wit",
    world: "sandboxed-channel",
});
```

Add:

```rust
pub struct WitChannelRuntime { /* shared Wasmtime engine/cache/config */ }
pub struct PreparedWitChannel { /* component + channel capabilities */ }
pub struct WitChannelHostState { /* emitted messages, workspace writes, attachments */ }
```

Support callbacks from `wit/channel.wit`:

- `on-start`
- `on-http-request`
- `on-poll`
- `on-respond`
- `on-broadcast`

Host imports map to Reborn services:

| WIT channel import | Reborn backing service |
| --- | --- |
| `log` | guest logs/events |
| `now-millis` | clock |
| `workspace-read` | Reborn channel workspace |
| `workspace-write` | Reborn channel workspace |
| `http-request` | WIT adapter -> shared runtime HTTP egress (#3085) -> `ironclaw_network` |
| `secret-exists` | `ironclaw_secrets` metadata/policy check |
| `emit-message` | Reborn channel message sink |
| `store-attachment-data` | Reborn attachment/channel state |

Compatibility requirements:

- Fresh instance per callback.
- Host-side state persists across callbacks.
- Durable workspace is host-managed.
- HTTP/secrets use the same shared runtime HTTP egress service (#3085) as tools.

## Phase 11: Add Reborn channel product integration

This is new-binary wiring only. Do not change V1 channel modules.

Implement Reborn equivalents for:

- Channel loader.
- Sidecar parser.
- `on-start` endpoint registration.
- Webhook router.
- Webhook secret/HMAC/signature validation through `ironclaw_secrets`.
- Polling loop.
- Websocket loop if still required.
- Pairing/owner routing equivalent if Reborn needs it.
- Broadcast metadata.
- Emitted message conversion into Reborn incoming messages.
- Durable channel workspace persistence.

Acceptance criteria:

- A current WIT channel fixture loads unchanged.
- `on-start` registers endpoint/poll configuration.
- `on-http-request` handles a webhook payload.
- `on-poll` can emit a message.
- `on-respond` can perform outbound HTTP through the kernel gateway.
- Durable channel workspace survives across callbacks.

## Phase 12: Add Reborn storage, trust, status, and cutover lifecycle

For production parity, Reborn needs lifecycle surfaces equivalent to the V1 external behavior, implemented natively:

- Binary hash verification.
- Installed/active/disabled/quarantined status.
- Trust policy integration.
- Install/activate/remove flows.
- Dev-mode discovery.
- DB-backed installed tools/channels when needed.

This phase should update docs and feature tracking:

- `FEATURE_PARITY.md`
- `docs/reborn/contracts/`
- setup docs if onboarding/config behavior changes

## Recommended PR sequence

### PR 1: Kernel network egress

- Extend `ironclaw_network` with production outbound HTTP.
- Add DNS/private-IP/redirect/streaming-limit tests.
- Preserve request-only egress accounting.

### PR 2: Kernel secrets leases and redaction

- Extend `ironclaw_secrets` for one-shot leases and redaction.
- Add missing required/optional credential tests.
- Prove raw secret values are not `Debug`/logged.

### PR 3: Shared Reborn runtime HTTP egress (#3085)

- Compose `ironclaw_secrets` + `ironclaw_network` once for WASM, Script, and host-mediated MCP HTTP.
- Add only thin runtime-specific adapters for WIT/Script/MCP request shapes.
- No V1 changes.
- Add tests proving Reborn WASM and at least one non-WASM runtime path go through the shared service.
- Define sandbox/proxy/fail-closed behavior for runtime modes that could otherwise open sockets directly.

### PR 4: Rewrite #3028 as WIT tool runtime

- Delete JSON ABI.
- Use `wit/tool.wit` and Wasmtime component model.
- Add `WitToolRuntime`.
- Add WIT tool compatibility tests.
- Use shared runtime HTTP egress (#3085) for HTTP/secrets.

### PR 5: Reborn WIT tool loader and dispatcher adapter

- Parse existing tool sidecars.
- Load existing WIT tool components.
- Register `RuntimeKind::Wasm`.
- Prove at least one existing/current-style WIT tool runs through Reborn dispatch.

### PR 6: Reborn WIT channel runtime

- Use `wit/channel.wit`.
- Execute channel callbacks.
- Add callback compatibility tests.
- Use the same shared runtime HTTP egress (#3085).

### PR 7: Reborn channel loader/router/polling

- Product integration for channels in the Reborn binary.
- Webhooks, polling, emitted messages, and durable channel workspace.

### PR 8: Reborn storage/trust/status

- Binary storage/hash verification.
- Active/disabled/quarantined lifecycle.
- Trust policy integration.
- Docs and feature parity updates.

## Immediate action for PR #3028

Change PR #3028's scope from:

```text
feat(reborn): add wasm runtime lane
```

to:

```text
feat(reborn): add WIT-compatible WASM tool runtime
```

The rewritten PR should remove the JSON ABI and land the first production-compatible tool runtime slice:

- Wasmtime component model.
- `wit/tool.wit` generated bindings.
- WIT tool metadata extraction.
- WIT tool execution.
- Kernel-backed HTTP/secrets host imports through shared runtime HTTP egress (#3085).
- No direct HTTP or app-level secret access from `ironclaw_wasm`.
- Hardened fuel/memory/timeout/resource behavior retained.

Channels should follow in a later PR.