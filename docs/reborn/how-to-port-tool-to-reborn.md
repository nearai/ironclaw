# How to port a v1 tool to Reborn

This guide helps maintainers decide how an IronClaw v1 tool should move onto the Reborn capability path.

The important distinction is that v1 used one `Tool` abstraction plus sidecar `*.capabilities.json` files for several different things. Reborn should keep those categories separate and describe them through Extension Manifest v2.

> Status note: this guide targets the manifest-v2 direction from issue #3537. If the target branch has not completed the v2 hard cutover yet, use the shapes here as the porting target, not as proof that every parser/runtime field is already implemented.

## Quick decision tree

Ask first: **is this tool host-owned, sandboxed extension code, a process wrapper, or an MCP adapter?**

| v1 source | Examples | Reborn target |
| --- | --- | --- |
| Host-owned built-in tool | `echo`, `time`, `json`, coding/file tools, memory, secrets, jobs, settings | Host-bundled `RuntimeKind::FirstParty` extension with registered first-party handlers |
| WASM API tool | `tools-src/web-search`, Gmail, Google Drive/Sheets/Docs/Slides, GitHub, Slack user tools | Installed or bundled `RuntimeKind::Wasm` extension using `wit/tool.wit` |
| Script or CLI wrapper | project-local helper, formatter/test runner, native CLI integration | `RuntimeKind::Script` extension with manifest-owned runner/command metadata |
| MCP integration | existing MCP server, stdio/http/sse adapter | `RuntimeKind::Mcp` extension |
| Simple REST integration | single HTTP API call with host-owned credential injection | Use WASM today; future candidate for `DeclarativeHttp` when that runtime lands |
| Webhook/inbound protocol tool | GitHub webhook receiver, Slack event callback, Telegram update processor | Usually not a tool; use ProductAdapter/channel porting guidance if it creates product/user inbound turns |

Rule of thumb:

- If IronClaw owns the implementation and it needs privileged host services, make it **FirstParty**.
- If a user or registry installs it and it can run in a sandbox, make it **WASM**.
- If it must execute a real process or CLI, make it **Script**.
- If the integration already speaks MCP, make it **MCP**.
- Do not make installed extensions self-declare `first_party` or `system`; source-aware host policy assigns effective trust.

## Why Reborn uses capabilities instead of v1 tools

V1 exposed model-visible tools directly from host registration:

```text
ToolRegistry
  -> Tool::schema() / Tool::execute(...)
  -> ad hoc approval/risk/auth/runtime behavior
```

Reborn should route every privileged action through one authority path:

```text
Extension Manifest v2
  -> ExtensionRegistry / capability descriptors
  -> Hot Capability Surface
  -> CapabilityHost authorization / approvals / obligations
  -> RuntimeDispatcher
  -> runtime adapter (FirstParty / WASM / Script / MCP)
```

This preserves one model-visible surface while keeping execution, authorization, trust, resource accounting, network policy, secret handling, and audit outside tool-specific code.

## Manifest v2 target shape

A Reborn tool port should target Extension Manifest v2:

```toml
schema_version = "reborn.extension_manifest.v2"

[extension]
id = "web-search"
name = "Web Search"
version = "0.2.0"
domain = "web"

[runtime]
kind = "wasm"
module = "wasm/web_search_tool.wasm"

[[capabilities]]
id = "web-search.search"
implements = ["web.search.v1"]
visibility = "model"
description = "Search the web using Brave Search."
effects = ["dispatch_capability", "network", "use_secret"]
default_permission = "ask"
required_host_ports = ["host.network.http", "host.secrets.inject"]
input_schema_ref = "schemas/search.input.v1.json"
output_schema_ref = "schemas/search.output.v1.json"
prompt_doc_ref = "prompt/search.md"
```

Important v2 rules:

- Capability IDs stay provider-prefixed: `<extension-id>.<operation>`.
- Manifests declare requested runtime/trust needs; they do not grant authority.
- `ManifestSource` comes from loader/install path, not TOML.
- Installed local/registry manifests may declare only `wasm`, `mcp`, or `script` runtimes.
- `first_party` and `system` runtimes are host-bundled only.
- `ironclaw.*` extension IDs are reserved for host-bundled extensions.
- Every capability uses extension-local `input_schema_ref` and `output_schema_ref`; no inline schemas, absolute paths, URLs, or `..` traversal.
- `prompt_doc_ref` is required only for `visibility = "model"` capabilities.
- Full manifests and JSON schemas are cold registry artifacts; hot loop context gets compact resolved tool surface only.

## Path A: port a host-owned built-in tool to FirstParty

Use this path for host-owned capabilities such as built-in coding tools, memory, secrets, jobs, settings/config, and small pure utilities.

### Target shape

```text
host-bundled extension package
  -> RuntimeKind::FirstParty
  -> FirstPartyCapabilityRegistry
  -> FirstPartyRuntimeAdapter
  -> CapabilityHost / RuntimeDispatcher path
```

The handler is host-owned code keyed by `CapabilityId`. Bundled TOML declares the surface; it is not authority by itself. Host composition must register the matching handler.

### What to preserve

- All callers still go through `CapabilityHost`.
- Effective first-party trust comes from host policy and source-aware validation.
- Handlers return normalized JSON output plus `ResourceUsage`.
- Handler failures use stable `RuntimeDispatchErrorKind`, not raw backend errors.
- Filesystem and storage access should come through scoped mounts/host ports, not raw host paths or DB handles.

### What not to do

- Do not expose raw `ToolRegistry` or v1 `Tool::execute` as the Reborn execution path.
- Do not let installed manifests request `first_party`/`system` runtime.
- Do not let handlers capture broad host services when a declared host port should provide a scoped view.
- Do not dump full first-party schemas or docs into every model turn.

### FirstParty checklist

- [ ] Define host-bundled extension ID and capability IDs.
- [ ] Set `runtime.kind = "first_party"` and `service = "..."` only in host-bundled source.
- [ ] Add capability `visibility`, schema refs, optional `implements`, and required host ports.
- [ ] Register matching `FirstPartyCapabilityHandler` values in `FirstPartyCapabilityRegistry`.
- [ ] Prove missing handler fails closed before side effects.
- [ ] Prove resources are reserved/reconciled/released exactly once.
- [ ] Add caller-level tests through `CapabilityHost` or host-runtime facade, not only handler unit tests.

## Path B: port a WASM API tool

Use this path for most existing `tools-src/*` integrations.

### Target shape

```text
tools-src/<tool>/
  -> wasm32-wasip2 component implementing wit/tool.wit
  -> Reborn extension manifest v2
  -> /system/extensions/<extension-id>/wasm/<module>.wasm
  -> RuntimeKind::Wasm
  -> host-mediated WASM imports
```

The existing WIT tool ABI remains the execution contract:

- `description() -> string`
- `schema() -> string`
- `execute(request) -> response`
- host imports: `log`, `now-millis`, `workspace-read`, `http-request`, `tool-invoke`, `secret-exists`

### Native core vs WASM wrapper

Pure tool logic may stay in normal Rust modules:

- parameter validation
- request construction
- response parsing/formatting
- bounded retry classification
- unit tests for URL/body rendering

The WASM wrapper owns:

- WIT exports
- conversion from JSON params into native types
- calls to host imports only, never direct network/secret/filesystem clients
- component smoke tests

Host runtime owns:

- component loading and metering
- scoped HTTP egress
- credential injection by approved handle/host policy
- secret existence checks and one-shot secret material staging
- resource accounting
- output limits/redaction

### Mapping from v1 sidecar JSON

| v1 `*.capabilities.json` field | Reborn target |
| --- | --- |
| `description` | `[extension].description` if available, plus capability `description` |
| `version` | `[extension].version` |
| `wit_version` | build/compat note; component must match host `wit/tool.wit` |
effects should include "network"; the porting report should list allowed targets for grants/host ports
| `http.credentials` | `effects += ["use_secret"]`; porting report should list secret handles and injection locations |
| `http.rate_limit` / timeouts / max body sizes | resource profile and runtime/host-port policy inputs where supported |
| `secrets.allowed_names` | `effects += ["use_secret"]`; host policy decides allowed handles |
| `workspace.allowed_prefixes` | `effects += ["read_filesystem"]`; map to scoped mounts/host ports |
| `tool_invoke.aliases` | avoid for first port; nested invocation must remain host-mediated and explicitly wired |
| `auth` / `setup` | onboarding metadata; keep as porting report until manifest v2 auth/install shape is finalized |
| `webhook` | usually ProductAdapter/channel path, not WASM tool invocation path |

### WASM checklist

- [ ] Build component with `wasm32-wasip2`.
- [ ] Keep tool code on `wit/tool.wit`; do not reintroduce pointer/length JSON ABI.
- [ ] Add manifest v2 with schema refs and `visibility`.
- [ ] Move exported JSON schema into `schemas/*.input.v1.json` and add output schema.
- [ ] Add short prompt doc for model-visible operations.
- [ ] Declare network/secret/filesystem effects; do not hide them in prose.
- [ ] Run component dispatch tests through Reborn WASM runtime with mocked host HTTP.
- [ ] Add redaction tests for provider errors and output shapes that might contain credentials.

## Path C: port a Script/CLI-backed tool

Use this path when the tool requires a real process, system binary, or language runtime that should not be embedded in WASM.

### Target shape

```toml
schema_version = "reborn.extension_manifest.v2"

[extension]
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
domain = "developer"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "project-tools.pytest"
implements = ["developer.test.run.v1"]
visibility = "model"
description = "Run the project pytest suite."
effects = ["dispatch_capability", "execute_code", "read_filesystem"]
default_permission = "ask"
input_schema_ref = "schemas/pytest.input.v1.json"
output_schema_ref = "schemas/pytest.output.v1.json"
prompt_doc_ref = "prompt/pytest.md"
```

### Script rules

- Runner/command/args come from the manifest, not model input.
- Invocation input is JSON over stdin.
- Output is bounded JSON over stdout.
- Host paths, environment variables, Docker flags, mounts, network, and secrets are not ambient.
- Network/secret access must use future mediated host ports or fail closed.

### Script checklist

- [ ] Verify the tool cannot be expressed as WASM first.
- [ ] Keep command and args manifest-owned.
- [ ] Define input/output schemas around JSON stdin/stdout.
- [ ] Set `default_permission = "ask"` for code execution or writes.
- [ ] Add output-size and wall-clock resource expectations.
- [ ] Test non-zero exit, timeout, invalid JSON, and output-limit failures.

## Path D: port an MCP tool/server

Use this path when an existing integration already speaks MCP.

### Target shape

```toml
schema_version = "reborn.extension_manifest.v2"

[extension]
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
domain = "developer"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/rpc"

[[capabilities]]
id = "github-mcp.search_issues"
implements = ["github.issues.search.v1"]
visibility = "model"
description = "Search GitHub issues through MCP."
effects = ["dispatch_capability", "network"]
default_permission = "ask"
input_schema_ref = "schemas/search-issues.input.v1.json"
output_schema_ref = "schemas/search-issues.output.v1.json"
prompt_doc_ref = "prompt/search-issues.md"
```

### MCP rules

- Manifest metadata selects transport and endpoint/command.
- Runtime input shapes MCP arguments only; it must not choose transport, URL, credentials, or network policy.
- HTTP/SSE must use host-mediated runtime egress.
- Stdio MCP remains process-backed and requires the process/sandbox posture to be ready before production use.

### MCP checklist

- [ ] Prefer HTTP/SSE only if host-mediated egress is configured.
- [ ] Treat stdio as blocked or follow-up until process-level egress controls are wired.
- [ ] Add output bound tests.
- [ ] Add session isolation tests keyed by scope/provider/url.
- [ ] Keep MCP server lifecycle/install/restart/monitoring out of the manifest parser.

## Path E: port a simple REST integration

If the tool only wraps one or a few HTTP endpoints, prefer this order:

1. WASM tool today.
2. Future `DeclarativeHttp` runtime once implemented.
3. FirstParty only if IronClaw owns the behavior and host policy requires host-owned code.

Do not add a bespoke host HTTP client for one provider. HTTP must go through Reborn network/egress policy and credential injection paths.

## Webhook caveat

A v1 tool sidecar may declare `webhook`, but webhooks often mean inbound product events rather than model-invoked tools.

Use ProductAdapter/channel porting guidance when the webhook:

- receives external actor messages/events;
- must verify protocol auth before parsing;
- maps external actor/conversation/event IDs;
- creates product inbound turns;
- sends delivery status back to an external protocol.

Keep the tool path only for host-authenticated control callbacks that dispatch normal capabilities.

## Inbound invocation recipe

For every Reborn capability invocation:

1. Validate input JSON against `input_schema_ref` before execution.
2. Resolve capability visibility through Hot Capability Surface; do not expose full manifest/schema text to the model.
3. Run `CapabilityHost` authorization and approval handling.
4. Prepare obligations: resources, network policy, secret injection, scoped mounts, redaction, output limits.
5. Dispatch to the selected runtime adapter.
6. Validate output JSON against `output_schema_ref`.
7. Redact and enforce output limits before publication.
8. Reconcile or release resources exactly once.

## Security checklist

- [ ] Manifest source is known (`HostBundled`, `InstalledLocal`, or `RegistryInstalled`) and used for validation.
- [ ] Installed extensions cannot request FirstParty/System trust or runtime.
- [ ] `ironclaw.*` IDs are host-bundled only.
- [ ] Every capability declares effects matching its side effects.
- [ ] Host ports are known to the host registry; unknown ports fail validation.
- [ ] Schema/doc refs are relative extension-local paths only.
- [ ] Installed `prompt_doc_ref` text is treated as untrusted model-facing content.
- [ ] Model-visible installed capabilities require explicit surface/admin policy.
- [ ] Runtime code never receives raw secrets unless a scoped host port explicitly allows it.
- [ ] Runtime code never constructs raw network clients that bypass host egress.
- [ ] Errors are stable and sanitized; no provider internals, raw tokens, raw host paths, or raw payload dumps.
- [ ] Output validation failures are audited with sanitized errors.

## Test checklist

For manifest parsing:

- [ ] valid v2 WASM manifest parses
- [ ] installed FirstParty/System request is rejected
- [ ] HostBundled FirstParty manifest parses but still requires matching handler
- [ ] `ironclaw.*` ID is rejected for installed manifests
- [ ] schema refs reject absolute paths, URLs, backslashes, and `..`
- [ ] model-visible capability without `prompt_doc_ref` is rejected
- [ ] unknown host port is rejected
- [ ] capability ID must be provider-prefixed

For runtime behavior:

- [ ] capability appears in Hot Capability Surface only when policy/trust/grants allow it
- [ ] input is validated before dispatch
- [ ] output is validated before publication
- [ ] resource reserve/reconcile/release paths are covered on success and failure
- [ ] network and secret access fail closed without prepared obligations
- [ ] redaction sentinels cover logs, errors, and output

For ported WASM tools:

- [ ] component builds as `wasm32-wasip2`
- [ ] component metadata/schema exports are valid
- [ ] mocked host HTTP proves expected method/URL/body/headers before real API use
- [ ] credentialed HTTP does not expose raw secret to guest memory/logs/output
- [ ] provider error bodies are bounded and sanitized

## Current dependencies and gaps

This guide is useful for planning and initial ports, but several production paths still depend on follow-up work:

- Extension Manifest v2 hard cutover in `ironclaw_extensions`.
- `ManifestSource`-aware validation for installed vs host-bundled manifests.
- Host-port vocabulary and scoped `HostPortView` handoff through `CapabilityHost`.
- Hot Capability Surface construction from `visibility`, `prompt_doc_ref`, profiles, trust, grants, and surface policy.
- Input/output schema validation before/after dispatch.
- Credential-account shaped injection and richer auth-blocked resume product flow.
- WASM tool credential compatibility for existing Google/GitHub/Brave/Slack sidecar files.
- Script network/secret/mount/artifact handoffs beyond the current restricted process posture.
- MCP stdio lifecycle and process-level egress controls.
- Future `DeclarativeHttp` runtime for simple REST tools.
- System runtime adapter remains separate from the current FirstParty adapter.

## References

- Issue: <https://github.com/nearai/ironclaw/issues/3537>
- `wit/tool.wit`
- `docs/reborn/contracts/extensions.md`
- `docs/reborn/contracts/host-runtime.md`
- `docs/reborn/contracts/host-api.md`
- `docs/reborn/contracts/capabilities.md`
- `docs/reborn/contracts/dispatcher.md`
- `docs/reborn/contracts/wasm.md`
- `docs/reborn/contracts/scripts.md`
- `docs/reborn/contracts/mcp.md`
- `docs/reborn/contracts/network.md`
- `src/tools/README.md`
- `src/tools/wasm/capabilities_schema.rs`
- `tools-src/web-search/` as the first recommended golden WASM tool port
