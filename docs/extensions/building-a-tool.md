---
title: How to implement a tool extension
description: "An implementation guide for IronClaw extension tools"
---

# How to implement a tool extension

This guide is for coding agents and engineers adding an IronClaw tool extension.

The guide is grounded in the current GitHub, GSuite, and Notion implementations:

- GitHub: bundled WASM capability provider under
  `crates/extensions/packages/github/`.
- GSuite: bundled WASM capability providers for Gmail, Calendar, Docs, Drive,
  Sheets, and Slides.
- Notion: bundled hosted HTTP MCP capability provider under
  `crates/ironclaw_first_party_extensions/assets/notion-mcp/`, with product
  auth / OAuth DCR wiring in composition.

## Success criteria

A tool extension is complete only when all of the following are true:

1. The extension package has a `schema_version = "reborn.extension_manifest.v3"` (or v2 for legacy manifests)
   manifest and every model-visible capability has schema, output schema, and
   prompt assets.
2. The manifest declares the correct runtime lane: `wasm`, `mcp`, or `script`.
3. (v3) Tools are declared with `[[tools]]` and inline `[[tools.credentials]]`.
   (v2 legacy) Tools are exposed through `ironclaw.capability_provider/v1` via the
   registry `[[host_api]]` path. Do not add or copy top-level
   `[[capabilities]]` declarations in either version.
4. The runtime code does not read raw secrets, create its own HTTP client for
   external provider calls, bypass approvals, or dispatch directly into the
   agent loop.
5. Network, credentials, approvals, and resource bounds are enforced by the
   host APIs and runtime services.
6. Tests cover manifest validation, runtime dispatch behavior, credential/auth
   gates, and caller-facing behavior through the runtime or lifecycle call site.

## Extension flow

Use this mental model before touching files:

```text
Extension package
  -> lifecycle/discovery materializes it into the extension registry
  -> ironclaw_extensions parses manifest surfaces and projects descriptors
  -> ironclaw_host_runtime publishes hot model-facing schemas/prompts
  -> model selects a visible capability
  -> ironclaw_capabilities performs authorization, approvals, obligations, run state
  -> host runtime selects the runtime adapter by RuntimeKind
  -> runtime executes through host-provided services
  -> host HTTP egress injects staged credentials and enforces network policy
  -> sanitized JSON output returns to the loop
```

Important ownership rule:

```text
ironclaw_extensions knows what can run.
runtime crates know how to run it.
authorization/approvals decide whether it may run.
host runtime/composition wires the concrete services.
```

Do not collapse those layers into a shortcut.

## Choose the runtime lane

Pick one lane first. Do not blend lanes to make a tool work.

| Lane | Use when | Current examples | Main files |
| --- | --- | --- | --- |
| WASM capability provider | Provider logic can run in a sandboxed component and use host HTTP egress. This is the default for provider tools. | GitHub, Gmail, Google Calendar, Google Drive, Google Docs, Google Sheets, Google Slides | `crates/ironclaw_first_party_extensions/assets/<id>/manifest.toml`, `schemas/`, `prompts/`, optional `wasm-src/` |
| Hosted HTTP MCP | The provider already exposes an MCP server and the host should lock egress to that endpoint. | Notion hosted MCP | `assets/<id>-mcp/manifest.toml`, schemas/prompts, `crates/ironclaw_extension_host/src/mcp.rs` only if adding a new host-bundled MCP policy shape |
| Product adapter | The extension receives external inbound events or product webhooks. This is not just a model-callable tool lane. | Slack/Telegram-style adapters, not the main focus of this guide | `crates/ironclaw_product/src/adapter_registry.rs` |
| Script | Sandboxed process/CLI capability. Use only when a process boundary is the product requirement. | Project tools / CLI-style tools | `crates/ironclaw_scripts` runtime path plus manifest runtime `script` |

For a new provider API like Linear, Jira, or a small internal SaaS API, start
with WASM unless you have a concrete reason not to.

## Crates to touch

Touch only the smallest set for your lane.

### Common extension package work

Usually touch:

- `crates/ironclaw_first_party_extensions/assets/<extension>/manifest.toml`
- `crates/ironclaw_first_party_extensions/assets/<extension>/schemas/<extension>/*.json`
- `crates/ironclaw_first_party_extensions/assets/<extension>/prompts/<extension>/*.md`
- `crates/ironclaw_extension_host/src/available_extensions.rs` only when adding
  a host-bundled available extension to the built-in install catalog.

Do not touch for ordinary tools:

- `crates/ironclaw_extensions/src/v2.rs` or `src/v3.rs`, unless changing the
  manifest contract itself.
- `crates/ironclaw_host_api/src/*`, unless adding a new shared host API type.
- `crates/ironclaw_capabilities`, unless changing authorization/approval
  orchestration for all capabilities.
- `crates/ironclaw_approvals`, unless changing approval lease semantics.
- `crates/ironclaw_secrets`, unless changing low-level secret storage/lease
  semantics.
- `crates/ironclaw_network`, unless changing global network policy/HTTP egress
  semantics.
- agent loop crates for tool-specific routing. Tool selection must come from the
  published capability surface, not hardcoded model-routing logic.

### WASM lane

Usually touch:

- `crates/extensions/packages/<extension>/wasm-src/`
- `crates/extensions/packages/<extension>/wasm/<tool>.wasm`
- the extension manifest, schemas, and prompts.
- `crates/ironclaw_extension_host/src/available_extensions.rs` to package
  the manifest, schemas, prompts, and WASM bytes if host-bundled.

Use as references:

- `crates/extensions/packages/github/wasm-src/src/lib.rs`
- `crates/extensions/packages/github/wasm-src/src/request.rs`
- `crates/ironclaw_host_runtime/src/wasm_credentials.rs`

Do not add a direct `reqwest`/HTTP client inside the WASM tool. Use the WIT host
HTTP import (`near::agent::host::http_request`) so the runtime can enforce egress,
inject staged credentials, and sanitize failures.

### Hosted MCP lane

Usually touch:

- `crates/extensions/packages/<provider>-mcp/manifest.toml`
- `schemas/<provider>/...`
- `prompts/<provider>/...`
- `crates/ironclaw_extension_host/src/available_extensions.rs` if
  host-bundled.

Use as references:

- `crates/ironclaw_first_party_extensions/assets/notion-mcp/manifest.toml`
- `crates/ironclaw_extension_host/src/mcp.rs`
- `crates/ironclaw_auth/src/engine/`
- composition provider wiring in `crates/ironclaw_reborn_composition/src/factory.rs`

Only touch `crates/ironclaw_extension_host/src/mcp.rs` if the hosted MCP
runtime policy needs a new generic rule. Notion already demonstrates the common
shape: HTTPS-only endpoint, exact host/path match, no URL credentials, no query,
no fragment, host-mediated egress, staged product-auth token.

### Auth/OAuth lane

Usually touch only when adding a new product-auth provider:

- `crates/ironclaw_auth` for provider/scopes/account-domain vocabulary when it
  must be shared and durable.
- `crates/ironclaw_auth/src/engine/` for generic recipe-driven OAuth/API-key
  exchange behavior.
- `crates/extensions/packages/<extension>/manifest.toml`
  for bundled first-party provider recipe data.
- `crates/ironclaw_reborn_composition/src/factory.rs` for composition-time
  provider recipe wiring.
- `crates/ironclaw_webui/src/product_auth/` only for product auth HTTP
  setup/callback route surfaces.

Do not create extension-local OAuth maps or store OAuth tokens in runtime code.
Credential accounts and secrets belong to `ironclaw_auth` /
`ironclaw_secrets` through composition.

## Files not to touch

For a tool extension, do not touch these:

- `src/agent/*` or loop strategy code to special-case your tool.
- `crates/ironclaw_llm/*` to teach the model your tool name.
- `crates/ironclaw_host_api` for one provider's fields.
- `crates/ironclaw_extensions/src/v2.rs` or `src/v3.rs` to allow a one-off manifest shortcut.
- `crates/ironclaw_network` to allow one provider host.
- `crates/ironclaw_secrets` to fetch one provider token.
- `crates/ironclaw_approvals` to make one write operation easier.

If your implementation appears to require one of these, stop and identify the
missing contract or composition seam first.

## Manifest structure

The current schema is `reborn.extension_manifest.v3`. v2 is also supported and parses through the same entry point, but new extensions should use v3.

v3 drops the `[[host_api]]` indirection in favor of first-class surface declarations. A tool extension uses:

```toml
schema_version = "reborn.extension_manifest.v3"
id = "example"
name = "Example"
version = "0.1.0"
description = "Example tools for IronClaw."
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/example_tool.wasm"

[[tools]]
id = "example.search"
description = "Search Example records."
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/example/search.input.v1.json"
output_schema_ref = "schemas/example/search.output.v1.json"
prompt_doc_ref = "prompts/example/search.md"

[[tools.credentials]]
handle = "example_runtime_token"
vendor = "example"
audience = { scheme = "https", host = "api.example.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

v3 surfaces are declared at the top level:
- `[[tools]]` — model-callable tools with inline credentials
- `[channel]` — at most one channel surface per extension
- `[mcp]` — hosted MCP server (mutually exclusive with `[channel]`)
- `[memory]` — memory provider surface
- `[auth.<vendor>]` — OAuth/API-key auth recipes

Extension IDs and capability IDs are authority-bearing:

- `id` must be lowercase ASCII letters/digits plus `_`, `-`, or `.`.
- Capability IDs are `<extension_id>.<capability_name>`.
- Do not use slashes, uppercase, raw host paths, or `..`.
- Registry extensions cannot claim effective first-party/system authority.
  Host composition decides effective trust.

### v2 compatibility: `host_api` (legacy)

v2 uses the capability-provider host API indirection. An equivalent v2 declaration looks like:

```toml
[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "example.search"
description = "Search Example records."
effects = ["dispatch_capability", "network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/example/search.input.v1.json"
output_schema_ref = "schemas/example/search.output.v1.json"
prompt_doc_ref = "prompts/example/search.md"
required_host_ports = ["host.runtime.http_egress"]
runtime_credentials = [
  { handle = "example_runtime_token", source = { type = "product_auth_account", provider = "example", setup = { kind = "oauth", scopes = ["records.read"] } }, provider_scopes = ["records.read"], audience = { scheme = "https", host_pattern = "api.example.com" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]
```

Do not use top-level `[[capabilities]]` for tool work. If a current
bundled manifest still has that shape, do not treat that file shape as a
reference. Treat it as migration debt.

### Capability fields (v3)

Required per model-visible tool capability in v3:

- `id`: stable `<extension>.<name>` capability ID.
- `description`: short, model-facing description.
- `effects`: accurate effects. Include `external_write` for provider writes,
  mutations, sends, deletes, comments, or workflow dispatches.
- `default_permission`: use `ask` for writes and high-risk reads; use `allow`
  only for low-risk read capabilities that policy deliberately permits.
- `visibility`: usually `model`.
- `input_schema_ref`: relative path to JSON schema.
- `output_schema_ref`: relative path to JSON schema.
- `prompt_doc_ref`: relative path to concise operation guidance.
- `credentials`: per-tool credential declarations (see below).

In v2, credentials are declared at the capability level with
`runtime_credentials` and network egress uses `required_host_ports`. In v3,
credentials are inline per tool with `[[tools.credentials]]` and host ports are
implied by the credential's audience.

Validation catches common mistakes:

- Credentials without `use_secret` in `effects` are rejected.
- Duplicate effects and duplicate credential handles are rejected.
- Credential audiences must be declared as HTTPS.
- Schema and prompt refs must be relative package paths, not absolute paths,
  URLs, backslash paths, or paths with `..`.

### Effects and approvals

Use effects as authorization inputs, not as documentation.

Common mapping:

- Read-only API call with credentials: `["dispatch_capability", "network",
  "use_secret"]`.
- Provider write: add `"external_write"`.
- Local filesystem read/write: use `read_filesystem`, `write_filesystem`,
  `delete_filesystem` as appropriate.
- Process/CLI work: use `execute_code` or `spawn_process` as appropriate.
- Money or irreversible financial actions: include `financial`.

`default_permission = "ask"` is the normal default for anything with
`external_write`, `financial`, local write/delete, process execution, approval
mutation, extension mutation, or budget mutation.

Approvals are resolved by `ironclaw_capabilities`, `ironclaw_approvals`, and run
state. Runtime code must return a normal runtime error when blocked; it must not
prompt the user, mint approval leases, or resume turns directly.

## Schemas and prompts

Schemas are part of the hot model-facing surface. They should make the desired
input shape obvious and reject ambiguous or unsafe input before side effects.

Follow these rules:

- Use JSON Schema object inputs with `additionalProperties: false` unless the
  upstream provider truly requires arbitrary JSON.
- Require the fields needed to construct one provider operation.
- Prefer provider-neutral names only when they are already established locally.
- Put path/ID/URL validation in runtime code too; schemas are not a security
  boundary.
- Output schemas may be provider raw JSON for compatibility, as GitHub and many
  Google WASM tools do, but typed output is better when the runtime owns the
  shape.

Prompt docs are lazy help metadata. Keep them operation-specific:

- What the tool does.
- Required identifiers.
- How to avoid common destructive mistakes.
- Any provider constraints the model should know.

Do not put secrets, host paths, environment assumptions, or V1 setup commands in
prompt docs.

## HTTP and network integration

Runtime code must use host-mediated HTTP:

- WASM tools call the WIT host HTTP import, as GitHub does through
  `near::agent::host::http_request`.
- Hosted MCP uses `McpHostHttpClient` with `McpRuntimeHttpAdapter` and a
  host-owned egress planner.

Do not:

- instantiate direct `reqwest` clients in runtime code for provider API calls;
- follow redirects yourself to bypass host policy;
- accept model-provided `Authorization`, cookie, API-key, or token headers;
- put credentials in URLs;
- widen global network policy for one extension.

Network policy belongs in host/runtime planning:

- WASM credential injection is derived from manifest descriptors in
  `crates/ironclaw_host_runtime/src/wasm_credentials.rs`.
- Hosted MCP policy is planned in
  `crates/ironclaw_extension_host/src/mcp.rs`.
- GSuite WASM tools should declare narrow credential audiences and use host
  HTTP egress for Google API hosts.
- Shared HTTP enforcement and redaction live in
  `crates/ironclaw_host_runtime/src/egress/` and `crates/ironclaw_network`.

Provider requests should set ordinary provider headers like `Accept`,
`Content-Type`, API version, and User-Agent in runtime code. Credential headers
must come from the credential declaration and host egress injection.

## Credentials and secrets

Secrets are opaque handles in manifests and host API types. Runtime code should
never see raw token material except as already-injected HTTP request data inside
the host egress boundary.

In v3, credentials are declared inline per tool with `[[tools.credentials]]`:

```toml
[[tools.credentials]]
handle = "github_runtime_token"
vendor = "github"
audience = { scheme = "https", host = "api.github.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

In v2, credentials use the `runtime_credentials` field at the capability level.

Important fields (v3):

- `handle`: extension/runtime-local credential handle. Keep it stable.
- `vendor`: provider account namespace, for example `github`, `google`, or
  `notion`. Must have a matching `[auth.<vendor>]` recipe in the manifest.
- `scopes`: scopes required for this capability. Used for account selection
  and scope mismatch checks.
- `audience`: exact HTTPS provider host the credential may be sent to.
- `injection`: header/query/path-placeholder injection target. Header is
  preferred.
- `required`: defaults to `true`.

In v2, the equivalent fields are `source`, `provider_scopes`, `audience` with
`host_pattern`, and `target`.

Credential flow:

```text
[[tools.credentials]]
  -> auth.vendor recipe provides OAuth/API-key setup
  -> authorization obligation for use_secret
  -> product-auth account selection or secret lease
  -> RuntimeSecretInjectionStore staging
  -> HostHttpEgressService injects once for matching capability + audience
  -> host strips/redacts sensitive request and response material
```

Do not call `SecretStore::put`, `lease_once`, or `consume` from an extension
runtime. Those are trusted setup/composition primitives, not tool APIs.

## Product auth and OAuth

Use product-auth account sources for provider accounts. Current patterns:

- GitHub uses provider `github` and injects a bearer token for
  `api.github.com`.
- GSuite uses provider `google`, OAuth scopes per capability, and host egress
  to Google API hosts.
- Notion uses provider `notion`, DCR/OAuth recipe data wired by composition, and a
  bearer token for `mcp.notion.com`.

For a new OAuth provider:

1. Add provider ID and shared scope vocabulary only if it must be shared across
   crates.
2. Add bundled first-party provider recipe data in
   `crates/extensions/packages/<extension>/manifest.toml`,
   keep `ironclaw_auth/src/engine/` generic, and wire the recipe resolver through
   composition provider wiring.
3. Wire OAuth start/callback through product-auth services, not an
   extension-local map.
4. Store access/refresh material as credential-account secret handles.
5. Declare per-capability scopes in `runtime_credentials`.
6. Ensure auth-required dispatch errors map to structured product-auth
   requirements instead of leaking provider or backend details.

Missing credentials should produce an auth-required gate, not a plain backend
failure and not a model-visible token prompt.

## WASM implementation pattern

WASM tools implement `crates/ironclaw_wasm/wit/tool.wit`:

```rust
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../ironclaw_wasm/wit/tool.wit",
});

struct ExampleTool;

impl exports::near::agent::tool::Guest for ExampleTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
                error: None,
            },
            Err(code) => exports::near::agent::tool::Response {
                output: None,
                error: Some(error_payload(&code)),
            },
        }
    }

    fn schema() -> String {
        schema::schema()
    }

    fn description() -> String {
        "Example tool. Credentials are injected only by host HTTP egress.".to_string()
    }
}

export!(ExampleTool);
```

Rules:

- Prefer operation selection from `req.context.capability_id`, as GitHub does.
  Do not let the model choose a hidden `action` that can mismatch the
  capability ID.
- Deserialize with unknown fields denied.
- Validate provider path segments, refs, IDs, pagination, and limits in runtime
  code before HTTP.
- Use host HTTP imports for provider calls.
- Return stable, sanitized error codes. Do not echo raw host egress errors,
  provider credentials, provider response bodies containing sensitive data, or
  raw backend messages.
- Keep schema and runtime input expectations in sync.

GitHub is the strongest current reference for this lane:

- `operation_comes_from_host_context_not_param_shape`
- `serde_rejects_unknown_fields_before_egress`
- `sanitizes_host_egress_errors_without_leaking_details`
- path/ref validation tests

## Hosted MCP implementation pattern

Hosted MCP packages declare runtime:

```toml
[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.notion.com/mcp"
```

For host-bundled hosted HTTP MCP, composition:

- accepts only HTTPS endpoint URLs;
- rejects userinfo, query strings, fragments, wrong scheme, wrong host, and
  wrong path;
- derives a locked network policy from the manifest endpoint;
- projects `runtime_credentials` to staged credential injections when the
  capability and endpoint audience match;
- uses `RuntimeHttpEgress` instead of ambient MCP HTTP clients.

Notion is the reference. Its manifest declares each MCP tool as a capability,
with per-tool schemas and prompts, and a product-auth `notion` credential for
`mcp.notion.com`.

Do not make a hosted MCP runtime call directly from an extension lifecycle or
agent-loop path. Let the MCP runtime and host egress planner own it.

## Packaging host-bundled extensions

Host-bundled extension packages are included in:

- `crates/ironclaw_extension_host/src/available_extensions.rs`

That file:

- includes manifest strings and WASM bytes;
- includes schema and prompt assets;
- builds `AvailableExtensionPackage`s;
- defines lifecycle summaries and onboarding text;
- materializes assets into `/system/extensions/<extension_id>/...`.

When adding a host-bundled package:

1. Add manifest/assets under
   `crates/extensions/packages/<extension>/`.
2. Add `include_str!` / `include_bytes!` entries in `available_extensions.rs`.
3. Add a package constructor like `github_package()` or `notion_mcp_package()`.
4. Add assets for every `input_schema_ref`, `output_schema_ref`, and
   `prompt_doc_ref`.
5. Add onboarding only if setup is needed.
6. Add tests that every manifest asset ref is packaged.

For non-bundled registry packages, do not add them to this host-bundled catalog.
They should be discovered from `/system/extensions/<id>/` through the same
manifest host API path.

## Publication to the model

Hot model-facing publication happens in:

- `crates/ironclaw_host_runtime/src/capability_catalog.rs`

It resolves input schema refs, output schema refs, and optional prompt docs
under the extension root. It does not grant authority and does not execute
runtime code.

Constraints to keep in mind:

- input/output schema files are bounded to 64 KiB;
- prompt docs are bounded to 16 KiB;
- schema files must parse as valid JSON Schema;
- only `visibility = "model"` capabilities enter the model-facing catalog.

If a tool does not appear to the model, inspect manifest visibility, lifecycle
activation, asset packaging, and schema validity before touching the agent loop.

## Approval and auth outcomes

A capability can stop before runtime dispatch for authorization or approval.
That is expected. Do not bypass it.

Approval path:

```text
CapabilityHost invokes
  -> authorization requires approval
  -> approval record is stored with invocation fingerprint
  -> run state marks blocked approval
  -> user resolves approval
  -> resolver issues scoped lease
  -> resume validates fingerprint
  -> runtime dispatch happens once
```

Auth-required path:

```text
runtime credential missing or scope-mismatched
  -> runtime/obligation returns auth-required context
  -> product-auth creates setup/OAuth/manual-token gate
  -> credential account stores access secret handle
  -> continuation resumes or the next invocation selects the account
```

Runtime code should produce typed/sanitized failures that map into these paths.
It should not serialize raw OAuth URLs, raw tokens, approval IDs, or provider
errors into model output.

## Tests to add

Minimum tests for a tool:

### Manifest and packaging

- manifest parses as `reborn.extension_manifest.v3` (or v2 for legacy manifests);
- capability IDs use the extension prefix;
- every capability has matching schema and prompt assets;
- credential capabilities include `use_secret`;
- write capabilities include `external_write` and default to `ask`;
- bundled package assets include every manifest ref;
- extension manifests use `[[tools]]` (v3) or `[[host_api]]` / `[capability_provider.tools]` (v2 legacy), never
  top-level `[[capabilities]]`.

Useful existing test areas:

- `crates/ironclaw_extensions/tests/manifest_v2_contract.rs`
- `crates/ironclaw_extensions/tests/manifest_v3_contract.rs`
- `crates/ironclaw_extension_host/src/available_extensions.rs` tests
- `crates/ironclaw_host_runtime/src/capability_catalog.rs` tests

### Runtime behavior

For WASM:

- operation comes from invocation context capability ID;
- unknown fields are rejected before egress;
- unsafe provider paths/refs are rejected;
- host egress errors are sanitized;
- auth status maps to auth-required rather than leaking backend detail;
- output-size/body-limit cases map to stable errors.

For hosted MCP:

- planner denies wrong provider, wrong host, HTTP scheme, wrong path, query,
  fragment, and URL userinfo;
- planner emits locked network policy for the canonical endpoint;
- manifest runtime credentials project to staged injections.

### Integration/caller-facing

Add a test through the actual call site that gates side effects:

- `CapabilityHost` or runtime adapter dispatch for capability invocation.
- Extension lifecycle install/readiness path for package publication.
- Product-auth setup/callback path for OAuth-backed credentials.

A helper-only test is not enough when a helper gates HTTP, DB writes, OAuth,
tool execution, or lifecycle readiness.

## Review checklist

Before opening a PR, verify:

- No V1 architecture paths were touched.
- No runtime code fetches raw secrets.
- No runtime code creates ambient external HTTP clients for provider calls.
- Every provider write has `external_write` and default `ask`.
- Every credential audience is HTTPS and as narrow as possible.
- Every schema/prompt ref is package-relative and packaged.
- Auth-required paths include provider/scopes/requester extension context.
- Error messages are sanitized and stable.
- Relevant docs/specs and `FEATURE_PARITY.md` were checked if behavior changed.
- Targeted tests pass.

## Concrete examples to copy

Copy these runtime, credential, and security patterns, not legacy manifest
shape. If one of these manifests still uses top-level `[[capabilities]]`, port
the semantics to v3 `[[tools]]` (or v2 `[[host_api]]` / `[capability_provider.tools]`
for legacy extensions) before extending it.

- GitHub WASM operation dispatch:
  `crates/extensions/packages/github/wasm-src/src/lib.rs`
- GitHub host HTTP request wrapper:
  `crates/extensions/packages/github/wasm-src/src/request.rs`
- GitHub manifest credential/effect semantics:
  `crates/extensions/packages/github/manifest.toml`
- Google Drive WASM OAuth scopes by operation:
  `crates/extensions/packages/google-drive/manifest.toml`
- Gmail and Google Calendar follow the bundled WASM GSuite manifest and runtime
  shape.
- Notion hosted MCP credential/effect semantics:
  `crates/extensions/packages/notion-mcp/manifest.toml`
- Hosted MCP egress planner:
  `crates/ironclaw_extension_host/src/mcp.rs`
- Notion OAuth provider wiring:
  `crates/ironclaw_reborn_composition/src/factory.rs`
- Hot capability catalog:
  `crates/ironclaw_host_runtime/src/capability_catalog.rs`
- Host HTTP egress service:
  `crates/ironclaw_host_runtime/src/egress/`
- Manifest v2 contract:
  `crates/ironclaw_extensions/src/v2.rs`

## Quick implementation checklist

1. Pick lane: WASM, hosted MCP, script, or product adapter.
2. Create package assets under `assets/<extension>/`.
3. Write manifest v3 with `[[tools]]` and inline `[[tools.credentials]]` (or v2
   `[[host_api]]` for legacy manifests) and make it flow through extension
   registry discovery/publication.
4. Add schemas and prompt docs for every model-visible capability.
5. Implement runtime code using host services only.
6. Declare credentials with narrow HTTPS audiences and provider scopes.
7. Add packaging/onboarding only if host-bundled.
8. Add manifest, packaging, runtime, auth/approval, and integration tests.
9. Run targeted tests.
10. Check docs/specs and `FEATURE_PARITY.md` for behavior-status updates.
