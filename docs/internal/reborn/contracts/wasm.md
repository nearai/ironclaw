# Reborn WASM runtime contract

The Reborn WASM runtime executes sandboxed extension components through the canonical component-model ABI declared in `crates/lanes/ironclaw_wasm/wit/tool.wit`.

## ABI

- Tool components implement world `near:agent/sandboxed-tool@0.4.1`.
- The host imports are the `near:agent/host@0.4.1` interface:
  - `log`
  - `now-millis`
  - `workspace-read`
  - `http-request`
  - `tool-invoke`
  - `secret-exists`
- Tool components export `near:agent/tool@0.4.1`:
  - `description() -> string`
  - `schema() -> string`
  - `execute(request) -> response`

The abandoned JSON pointer/length ABI (`alloc`, `invoke_json`, `output_ptr`, `output_len`, and runtime-specific HTTP imports such as `http_request_utf8`) is not part of Reborn.

### Typed tool response (0.4.1)

`near:agent@0.4.1` replaced the untyped JSON-string `execute` return value with
a typed `wit-result`-shaped `variant response`:

- `success(string)` — the JSON-encoded tool output.
- `failure(guest-failure)` — a structured failure record instead of an
  ad-hoc error string, with:
  - `kind: error-kind` — a closed enum (`auth-required`, `input`,
    `output-too-large`, `executor`, `network-denied`, `client`,
    `operation-failed`) that mirrors the host-side
    `RuntimeDispatchErrorKind` mapping 1:1, plus `auth-required` mapping to
    `DispatchError::AuthRequired`.
  - `code: option<string>` — a stable, short provider/tool error identifier
    when the guest has one (e.g. a Slack `channel_not_found` code).
  - `message: option<string>` — human-readable failure detail.

`code` and `message` are free-text carriers: the host scrubs both for
secret-shaped values at the sandbox-exit chokepoint before either is visible
outside the guest. Each scrubbed failure field and each buffered guest log
record is UTF-8-safely bounded to 4 KiB at that boundary;
`ironclaw_host_runtime` redacts and applies the canonical
`MODEL_DIAGNOSTIC_MAX_BYTES` bound before tracing, then narrows typed provider
messages to 2 KiB before they enter dispatch metadata and are re-validated at
the model-visible seam.

`near:agent@0.4.1` typed responses are the sole supported tool contract.
Components targeting another WIT contract version fail closed during
instantiation with `WasmError::UnsupportedContract`; unrelated or unknown
imports remain `WasmError::InstantiationFailed`.

The same ABI revision makes `host.http-request` failures typed. Its coarse
`http-error-kind` mirrors the guest failure categories; provider-specific
detail remains optional `code`/`message` data, and `request-sent` exists only
for resource accounting. Adding WASM, MCP, CLI, or native extensions does not
widen this enum: non-WASM lanes continue normalizing through the canonical
host dispatch contracts.

## Runtime invariants

- Compile once, instantiate a fresh component instance for every execution.
- Apply fuel, epoch-timeout, memory, table, and instance limits to every metadata and execution call.
- Allow component-model tools to instantiate multiple internal memories when needed, but enforce `SandboxLimits::memory_bytes` as an aggregate per-execution memory budget across all memories.
- Treat WIT metadata as the source of runtime compatibility: `description()` and `schema()` are called through generated Wasmtime component bindings.
- Keep V1 `src/tools/wasm/*` and `src/channels/wasm/*` as compatibility references only; Reborn is a separate binary path.

## Host capability seams

All host capabilities are injected through explicit Rust seams. The default host is fail-closed:

- HTTP egress returns an unavailable error unless a host implementation is injected.
- Workspace reads return `None` unless a workspace implementation is injected.
- Secret access is existence-only and returns `false` unless a secret implementation is injected.
- Nested tool invocation returns unavailable unless a tool implementation is injected.

Production HTTP is wired through `WasmRuntimeHttpAdapter`, a thin adapter from the WIT `http-request` import to the shared Reborn `RuntimeHttpEgress` service. `ironclaw_wasm` does not implement direct HTTP clients, DNS resolution, SSRF checks, credential injection, or response streaming. Host composition supplies scope, capability id, response limits, and a request-scoped credential provider before constructing the adapter; the shared runtime egress service consumes the scoped/capability `ApplyNetworkPolicy` handoff from `NetworkObligationPolicyStore` and passes that host-approved policy to `ironclaw_network`. Credential providers must derive credential injections from the actual method/URL/headers being requested, not from guest input alone or from a reusable adapter-wide grant. Shared runtime egress owns request leak checks, request sensitive-header handling, policy enforcement, credential injection, response redaction, and sanitized runtime-visible errors. The WASM adapter additionally strips sensitive response headers before encoding the WIT `headers-json` object using the shared runtime sensitive-header vocabulary. Because the WIT ABI defines headers as a JSON object string, duplicate non-sensitive response header names are combined case-insensitively with comma separators at this boundary after shared egress has already applied response safety checks. The WASM runtime applies the WIT HTTP default timeout when `timeout-ms` is omitted, caps it to the remaining execution deadline, forwards that cap through `RuntimeHttpEgress`, and reports a timeout if a host import returns after that deadline; injected synchronous host implementations must still honor the supplied timeout because they cannot be safely preempted mid-call.

Production `secret-exists` is wired through `StagedWasmHostSecrets` (`ironclaw_host_runtime::services::wasm_secrets`), a per-invocation view over the staged secret injection store: `exists(name)` returns `true` exactly when authorization leased and staged non-empty credential material for this invocation's `(scope, capability_id, handle)`, and `false` otherwise (fail-closed). The read is non-consuming (`clone_material`), so the probe does not steal the material the HTTP egress still needs. Credential staging rejects empty resolved material as `AuthRequired` so a configured-but-blank credential surfaces the typed re-auth signal instead of an opaque guest failure.

## Network accounting

`ResourceUsage.network_egress_bytes` counts outbound request body bytes only. Response body limits and response scanning are separate host-egress responsibilities and must not be recorded as egress usage. If the host reports that a request was sent but later failed during response handling, the request body still counts as egress; fail-closed denials before send count zero. Execution failures preserve the usage/log snapshot collected before the failure so callers can reconcile sent egress even when the guest traps.
