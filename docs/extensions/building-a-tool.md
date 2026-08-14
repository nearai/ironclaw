---
title: How to implement a Reborn tool extension
description: "A Reborn-only implementation guide for IronClaw extension tools"
---

# How to implement a Reborn tool extension

This guide is for coding agents and engineers adding an IronClaw Reborn
extension tool. It is intentionally Reborn-only. Do not use V1 extension,
native-extension, pending-OAuth-map, or legacy tool-router patterns when
following this document.

The guide is grounded in the current GitHub, GSuite, and Notion implementations:

- GitHub: bundled WASM capability provider under
  `crates/extensions/packages/github/`.
- GSuite: bundled WASM capability providers for Docs, Drive, Sheets, and
  Slides; Gmail and Calendar are bundled `first_party` runtimes sharing the
  same `google` vendor.
- Notion: bundled hosted HTTP MCP capability provider under
  `crates/extensions/packages/notion-mcp/`, with product
  auth / OAuth DCR wiring in Reborn composition.

## Success criteria

A Reborn tool extension is complete only when all of the following are true:

1. The extension package has a `schema_version = "reborn.extension_manifest.v3"`
   manifest and every model-visible capability has schema, output schema, and
   prompt assets.
2. The manifest declares exactly one implementation: `[runtime]` with
   `kind = "wasm"` or `kind = "first_party"`, or a `[mcp]` section for
   hosted-MCP extensions.
3. The manifest exposes tools as `[[tools]]` entries, each carrying an
   `origin_gate_matrix`. Do not use the legacy `[[host_api]]` /
   `[capability_provider.tools]` or top-level `[[capabilities]]` shapes.
4. The runtime code does not read raw secrets, create its own HTTP client for
   external provider calls, bypass approvals, or dispatch directly into the
   agent loop.
5. Network, credentials, approvals, and resource bounds are enforced by the
   Reborn host APIs and runtime services.
6. Tests cover manifest validation, runtime dispatch behavior, credential/auth
   gates, and caller-facing behavior through the runtime or lifecycle call site.

## Reborn extension flow

Use this mental model before touching files:

```text
Extension package
  -> lifecycle/discovery materializes it into the extension registry
  -> ironclaw_extension_registry parses the v3 manifest and projects descriptors
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
ironclaw_extension_registry knows what can run.
runtime crates know how to run it.
authorization/approvals decide whether it may run.
host runtime/composition wires the concrete services.
```

Do not collapse those layers into a shortcut.

## Choose the runtime lane

Pick one lane first. Do not blend lanes to make a tool work.

| Lane | Use when | Current examples | Main files |
| --- | --- | --- | --- |
| WASM capability provider | Provider logic can run in a sandboxed component and use host HTTP egress. This is the default for provider tools. | GitHub, Google Drive, Google Docs, Google Sheets, Google Slides (Gmail and Google Calendar are `first_party` runtimes, not WASM) | `crates/extensions/packages/<id>/manifest.toml`, `schemas/`, `prompts/`, optional `wasm-src/` |
| Hosted HTTP MCP | The provider already exposes an MCP server and the host should lock egress to that endpoint. | Notion hosted MCP | `crates/extensions/packages/<id>-mcp/manifest.toml`, schemas/prompts, `crates/extensions/ironclaw_extension_host/src/mcp.rs` only if adding a new host-bundled MCP policy shape |
| Channel surface (formerly "product adapter") | The extension receives external inbound events or product webhooks. This is not just a model-callable tool lane. | Slack/Telegram channel surfaces, not the main focus of this guide | the package's `[channel]` manifest section + its `ChannelAdapter` (`crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`); worked examples `crates/extensions/packages/{slack,telegram}`; see the `reborn-extension-surfaces` skill |

There is no `script` runtime kind in v3 manifests. Process/CLI work goes
through the built-in process sandbox capability
(`system.process_sandbox.run`), not an extension runtime lane.

For a new provider API like Linear, Jira, or a small internal SaaS API, start
with WASM unless you have a concrete reason not to.

## Crates to touch

Touch only the smallest set for your lane.

### Common extension package work

Usually touch:

- `crates/extensions/packages/<extension>/manifest.toml`
- `crates/extensions/packages/<extension>/schemas/<extension>/*.json`
- `crates/extensions/packages/<extension>/prompts/<extension>/*.md`
- when adding a host-bundled extension to the built-in install catalog: a
  package module `crates/extensions/ironclaw_extension_support/src/packages/<extension>.rs`
  plus its row in `PACKAGES` in `.../src/packages/mod.rs`. Do not register
  assets in composition; the old `available_extensions.rs` home (now
  `crates/extensions/ironclaw_extension_host/src/available_extensions.rs`) is
  being dissolved.

Do not touch for ordinary tools:

- `crates/extensions/ironclaw_extension_registry/src/v3.rs` (or the legacy
  `src/v2.rs`), unless changing the manifest contract itself.
- `crates/contracts/ironclaw_host_api/src/*`, unless adding a new shared host API type.
- `crates/kernel/ironclaw_capabilities`, unless changing authorization/approval
  orchestration for all capabilities.
- `crates/kernel/ironclaw_approvals`, unless changing approval lease semantics.
- `crates/substrates/ironclaw_secrets`, unless changing low-level secret storage/lease
  semantics.
- `crates/substrates/ironclaw_network`, unless changing global network policy/HTTP egress
  semantics.
- agent loop crates for tool-specific routing. Tool selection must come from the
  published capability surface, not hardcoded model-routing logic.

### WASM lane

Usually touch:

- `crates/extensions/packages/<extension>/wasm-src/`
- `crates/extensions/packages/<extension>/wasm/<tool>.wasm`
- the extension manifest, schemas, and prompts.
- the package module in `crates/extensions/ironclaw_extension_support/src/packages/`
  to embed the manifest, schemas, prompts, and WASM bytes if host-bundled.

Use as references:

- `crates/extensions/packages/github/wasm-src/src/lib.rs`
- `crates/extensions/packages/github/wasm-src/src/request.rs`
- `crates/kernel/ironclaw_host_runtime/src/wasm_credentials.rs`

Do not add a direct `reqwest`/HTTP client inside the WASM tool. Use the WIT host
HTTP import (`near::agent::host::http_request`) so Reborn can enforce egress,
inject staged credentials, and sanitize failures.

### Hosted MCP lane

Usually touch:

- `crates/extensions/packages/<provider>-mcp/manifest.toml`
- `schemas/<provider>/...`
- `prompts/<provider>/...`
- the package module in `crates/extensions/ironclaw_extension_support/src/packages/`
  if host-bundled.

Use as references:

- `crates/extensions/packages/notion-mcp/manifest.toml`
- `crates/extensions/ironclaw_extension_host/src/mcp.rs`
- `crates/domains/ironclaw_auth/src/engine/`
- composition provider wiring in `crates/app/ironclaw_composition/src/factory.rs`

Only touch `crates/extensions/ironclaw_extension_host/src/mcp.rs` if the hosted MCP
runtime policy needs a new generic rule. Notion already demonstrates the common
shape: HTTPS-only endpoint, exact host/path match, no URL credentials, no query,
no fragment, host-mediated egress, staged product-auth token.

### Auth/OAuth lane

Usually touch only when adding a new product-auth provider:

- `crates/domains/ironclaw_auth` for provider/scopes/account-domain vocabulary when it
  must be shared and durable.
- `crates/domains/ironclaw_auth/src/engine/` for generic recipe-driven OAuth/API-key
  exchange behavior.
- `crates/extensions/packages/<extension>/manifest.toml`
  for bundled first-party provider recipe data.
- `crates/app/ironclaw_composition/src/factory.rs` for composition-time
  provider recipe wiring.
- `crates/product/ironclaw_webui/src/product_auth/` only for product auth HTTP
  setup/callback route surfaces.

Do not create extension-local OAuth maps or store OAuth tokens in runtime code.
Credential accounts and secrets belong to `ironclaw_auth` /
`ironclaw_secrets` through Reborn composition.

## Files not to touch

For a normal extension, do not touch these:

- Reborn loop strategy code (`crates/loop/`) to special-case your tool.
- `crates/domains/ironclaw_llm/*` to teach the model your tool name.
- `crates/contracts/ironclaw_host_api` for one provider's fields.
- `crates/extensions/ironclaw_extension_registry/src/v2.rs` to allow a one-off manifest shortcut.
- `crates/substrates/ironclaw_network` to allow one provider host.
- `crates/substrates/ironclaw_secrets` to fetch one provider token.
- `crates/kernel/ironclaw_approvals` to make one write operation easier.

If your implementation appears to require one of these, stop and identify the
missing Reborn contract or composition seam first.

## Manifest v3 structure

All Reborn packages use:

```toml
schema_version = "reborn.extension_manifest.v3"
id = "example"
name = "Example"
version = "0.1.0"
description = "Example tools for Reborn."
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/example_tool.wasm"
```

Extension IDs and capability IDs are authority-bearing:

- `id` must be lowercase ASCII letters/digits plus `_`, `-`, or `.`.
- Capability IDs are `<extension_id>.<capability_name>`.
- Do not use slashes, uppercase, raw host paths, or `..`.
- Registry extensions cannot claim effective first-party/system authority.
  Host composition decides effective trust.

### All tool extensions: use `[[tools]]`

Publish each model-visible tool as a `[[tools]]` entry with its credentials
as `[[tools.credentials]]` blocks and an `[auth.<vendor>]` recipe for every
referenced credential vendor:

```toml
[[tools]]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "example.search"
description = "Search Example records."
effects = ["network", "use_secret"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/example/search.input.v1.json"
output_schema_ref = "schemas/example/search.output.v1.json"
prompt_doc_ref = "prompts/example/search.md"

[[tools.credentials]]
handle = "example_runtime_token"
vendor = "example"
scopes = ["records.read"]
audience = { scheme = "https", host = "api.example.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[auth.example]
method = "oauth2_code"
display_name = "Example account"
authorization_endpoint = "https://example.com/oauth/authorize"
token_endpoint = "https://example.com/oauth/token"
scopes = ["records.read"]

[auth.example.token_response]
access_token = "/access_token"
```

`token_response` is required for `oauth2_code` recipes — it maps JSON
pointers in the provider's token response (at minimum `access_token`; add
`refresh_token` / `expires_in` for expiring tokens, as Notion does).

Host ports are derived from effects in v3 — do not declare
`required_host_ports`. The derived entries are validation vocabulary checked
against the host's `HostPortCatalog` allowlist; concrete host-port adapters
are constructed by host-runtime services only after authorization and
obligation preparation, never from the manifest. The worked `oauth2_code`
examples (token-response captures, identity maps) are
`crates/extensions/packages/slack/manifest.toml` and
`crates/extensions/packages/notion-mcp/manifest.toml`; the worked `api_key`
example (form fields plus a validation probe) is in
`crates/extensions/packages/github/manifest.toml`.

Do not use the legacy `[[host_api]]` / `[capability_provider.tools]` shape or
top-level `[[capabilities]]` for new work. The registry still parses
already-installed v2 manifests for compatibility, but authoring is v3-only;
port a v2 file to `[[tools]]` when touching that extension.

### Origin gate matrix

Every `[[tools]]` entry (and the `[mcp]` section) declares an
`origin_gate_matrix`: the per-origin approval-gate policy for who may invoke
the capability. Origins are `loop_run` (the model during an agent run),
`product` (a direct user gesture in the product), and `automation` (triggers,
cron, background jobs). Policies, from `OriginGatePolicy` in
`crates/contracts/ironclaw_host_api/src/capability.rs`:

- `forbidden` — the origin may not invoke the capability at all. This is the
  default for every omitted origin, so a tool without a matrix cannot be
  invoked from any origin.
- `ask_always` — every invocation gates; persistent grants are never honored.
- `gated_unless_granted` — gates unless a scoped persistent/policy grant
  covers it. The normal choice for provider tools invoked by the model.
- `consent_sufficient` — the origin's own gesture is the consent evidence
  (`product` only; never valid for `loop_run` or `automation`).
- `ungated` — no approval gate. For `loop_run` this requires a reviewed
  allowlist entry (`UNGATED_LOOP_RUN_CAPABILITIES`); additions are a security
  review, not a manifest edit.

The field is structurally optional in the parser, but shipped packages are
required to declare a `loop_run` policy on every capability by the
architecture ratchet
(`crates/app/ironclaw_architecture_tests/tests/reborn_origin_gate_matrix_ratchet.rs`).
The common provider-tool shape is
`{ loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }`.

### Capability fields

Required per model-visible `[[tools]]` entry:

- `origin_gate_matrix`: per-origin gate policy (previous section).
- `id`: stable `<extension>.<name>` capability ID.
- `description`: short, model-facing description.
- `effects`: accurate effects. Include `external_write` for provider writes,
  mutations, sends, deletes, comments, or workflow dispatches.
- `default_permission`: use `ask` for writes and high-risk reads; use `allow`
  only for low-risk read capabilities that policy deliberately permits.
- `visibility`: usually `model` (the default).
- `input_schema_ref`: relative path to JSON schema. Required unless the tool
  binds a `standard_op`, which supplies the host-canonical schemas.
- `output_schema_ref`: relative path to JSON schema. Optional — omit it when
  the tool has no structured output contract (the gmail package declares
  none).
- `prompt_doc_ref`: relative path to concise operation guidance.
- `[[tools.credentials]]`: declare every credential the runtime may receive.

Optional bounds: `network_targets`, `max_egress_bytes`, `resource_profile`.

Validation catches common mistakes:

- Credentials without `use_secret` in `effects` are rejected. This includes
  product-auth account credentials: product auth selects/refreshes the account,
  but runtime dispatch still uses a host-staged access-secret handle.
- A credential `vendor` with no matching `[auth.<vendor>]` recipe is rejected,
  and an `[auth.<vendor>]` recipe no credential references is rejected.
- Credential audiences must be HTTPS with a literal host — wildcards are
  rejected in v3.
- Duplicate effects and duplicate credential handles are rejected.
- Unknown fields are rejected throughout the manifest, with two deliberate
  exemptions: the root `[metadata]` table is free-form authoring metadata
  (ignored), and extra keys in an `[auth.<vendor>].identity` map are named
  identity-claim pointers, not typos.
- Schema and prompt refs must be relative package paths, not absolute paths,
  URLs, backslash paths, or paths with `..`.

### Effects and approvals

Use effects as authorization inputs, not as documentation.

Common mapping:

- Read-only API call with credentials: `["network", "use_secret"]`.
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
  `crates/kernel/ironclaw_host_runtime/src/wasm_credentials.rs`.
- Hosted MCP policy is planned in
  `crates/extensions/ironclaw_extension_host/src/mcp.rs`.
- GSuite WASM tools should declare narrow credential audiences and use host
  HTTP egress for Google API hosts.
- Shared HTTP enforcement and redaction live in
  `crates/kernel/ironclaw_host_runtime/src/egress/` and `crates/substrates/ironclaw_network`.

Provider requests should set ordinary provider headers like `Accept`,
`Content-Type`, API version, and User-Agent in runtime code. Credential headers
must come from `[[tools.credentials]]` and host egress injection.

## Secrets and runtime credentials

Secrets are opaque handles in manifests and host API types. Runtime code should
never see raw token material except as already-injected HTTP request data inside
the host egress boundary.

Declare a `[[tools.credentials]]` block for every credential a tool may
receive. The credential's `vendor` names the credential authority; the
matching `[auth.<vendor>]` recipe in the same manifest defines how accounts
are set up (OAuth or API key). Host egress injects the selected account's
access-secret handle at dispatch time:

```toml
[[tools.credentials]]
handle = "github_runtime_token"
vendor = "github"
audience = { scheme = "https", host = "api.github.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

Important fields:

- `handle`: extension/runtime-local credential handle. Keep it stable.
- `vendor`: credential-authority namespace, for example `github`, `google`,
  or `notion`. Several extensions may share one vendor (gmail, drive, and
  calendar all use `google`). This is not the extension id.
- `scopes`: scopes required for this capability. Used for account selection
  and scope mismatch checks.
- `audience`: exact HTTPS provider host (literal, no wildcards) the credential
  may be sent to. Optional `port`.
- `injection`: header/query/path-placeholder injection target. Header is
  preferred.
- `required`: defaults to `true`.

Credential flow:

```text
manifest [[tools.credentials]]
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
2. Add the `[auth.<vendor>]` recipe in
   `crates/extensions/packages/<extension>/manifest.toml` (`method =
   "oauth2_code"` or `"api_key"`), and keep `ironclaw_auth/src/engine/`
   generic — the host engine runs the recipe; there is no per-vendor auth
   code.
3. Wire OAuth start/callback through product-auth services, not an
   extension-local map.
4. Store access/refresh material as credential-account secret handles.
5. Declare per-capability scopes on each `[[tools.credentials]]` block.
6. Ensure auth-required dispatch errors map to structured product-auth
   requirements instead of leaking provider or backend details.

Missing credentials should produce an auth-required gate, not a plain backend
failure and not a model-visible token prompt.

## WASM implementation pattern

WASM tools implement `crates/lanes/ironclaw_wasm/wit/tool.wit`:

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
        "Example Reborn tool. Credentials are injected only by host HTTP egress.".to_string()
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

A hosted-MCP extension declares a top-level `[mcp]` section **instead of**
`[runtime]` — the two are mutually exclusive:

```toml
[mcp]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
server = "https://mcp.notion.com/mcp"
namespace = "notion"
max_tools = 256
default_permission = "ask"
effects = ["network", "use_secret", "external_write"]

[[mcp.credentials]]
handle = "mcp_notion_access_token"
vendor = "notion"
scopes = []
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

`namespace` must equal the extension id, and `max_tools` must be at least 1.
Tools are discovered from the server's live catalog; a package may
additionally pin static `[[tools]]` entries beside `[mcp]` that a successful
discovery replaces (worked example: `crates/extensions/packages/nearai-mcp/manifest.toml`).

For host-bundled hosted HTTP MCP, Reborn composition:

- accepts only HTTPS endpoint URLs;
- rejects userinfo, query strings, fragments, wrong scheme, wrong host, and
  wrong path;
- derives a locked network policy from the manifest endpoint;
- projects `[[mcp.credentials]]` to staged credential injections when the
  capability and endpoint audience match;
- uses `RuntimeHttpEgress` instead of ambient MCP HTTP clients.

Notion is the reference (`crates/extensions/packages/notion-mcp/manifest.toml`):
one `[mcp]` section, an `[auth.notion]` OAuth recipe, and a bearer credential
for `mcp.notion.com`.

Do not make a hosted MCP runtime call directly from an extension lifecycle or
agent-loop path. Let the MCP runtime and host egress planner own it.

## Packaging host-bundled extensions

Every host-bundled integration is a self-contained package directory
`crates/extensions/packages/<extension>/` (manifest + schemas + prompts + any
WASM) beside one package module
`crates/extensions/ironclaw_extension_support/src/packages/<extension>.rs`.

The package module:

- embeds the manifest, schema, prompt, and WASM assets via `include_str!` /
  `include_bytes!`;
- defines lifecycle summaries and onboarding text;
- is collected through its row in `PACKAGES` in
  `crates/extensions/ironclaw_extension_support/src/packages/mod.rs`.

Composition consumes packages as opaque bundles and never names one; the
binary alone links the concrete channel-adapter crates (slack, telegram) and
builds their binding table.
Do not register package assets in composition — the old
`available_extensions.rs` home is being dissolved.

When adding a host-bundled package:

1. Add manifest/assets under
   `crates/extensions/packages/<extension>/`.
2. Add the package module in
   `crates/extensions/ironclaw_extension_support/src/packages/<extension>.rs`
   and its row in `PACKAGES` in `.../src/packages/mod.rs`.
3. Add assets for every `input_schema_ref`, `output_schema_ref`, and
   `prompt_doc_ref`.
4. Add onboarding only if setup is needed.
5. Add tests that every manifest asset ref is packaged.

For non-bundled registry packages, do not add them to this host-bundled catalog.
They should be discovered from `/system/extensions/<id>/` through the same
manifest host API path.

## Publication to the model

Hot model-facing publication happens in:

- `crates/kernel/ironclaw_host_runtime/src/capability_catalog.rs`

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

Minimum tests for a Reborn tool:

### Manifest and packaging

- manifest parses as `reborn.extension_manifest.v3`;
- capability IDs use the extension prefix;
- every capability declares an `origin_gate_matrix` with a `loop_run` policy;
- every capability has matching schema and prompt assets;
- credential capabilities include `use_secret` and every credential vendor has
  an `[auth.<vendor>]` recipe;
- write capabilities include `external_write` and default to `ask`;
- bundled package assets include every manifest ref;
- extension manifests use `[[tools]]` entries, never the legacy
  `[[host_api]]` / `[capability_provider.tools]` or top-level
  `[[capabilities]]` shapes.

Useful existing test areas:

- `crates/extensions/ironclaw_extension_registry/tests/manifest_v3_contract.rs`
- `crates/app/ironclaw_architecture_tests/tests/reborn_origin_gate_matrix_ratchet.rs`
- `crates/kernel/ironclaw_host_runtime/src/capability_catalog.rs` tests

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
shape. If a manifest you encounter still uses the legacy v2 shapes
(`[[host_api]]` / `[capability_provider.tools]` or top-level
`[[capabilities]]`), port the semantics into v3 `[[tools]]` entries before
extending it.

- GitHub WASM operation dispatch:
  `crates/extensions/packages/github/wasm-src/src/lib.rs`
- GitHub host HTTP request wrapper:
  `crates/extensions/packages/github/wasm-src/src/request.rs`
- GitHub manifest credential/effect semantics:
  `crates/extensions/packages/github/manifest.toml`
- Google Drive WASM OAuth scopes by operation:
  `crates/extensions/packages/google-drive/manifest.toml`
- Gmail and Google Calendar are bundled `first_party` runtimes sharing the
  `google` vendor — a reference for the first-party lane, not for WASM.
- Notion hosted MCP credential/effect semantics:
  `crates/extensions/packages/notion-mcp/manifest.toml`
- Hosted MCP egress planner:
  `crates/extensions/ironclaw_extension_host/src/mcp.rs`
- Notion OAuth provider wiring:
  `crates/app/ironclaw_composition/src/factory.rs`
- Hot capability catalog:
  `crates/kernel/ironclaw_host_runtime/src/capability_catalog.rs`
- Host HTTP egress service:
  `crates/kernel/ironclaw_host_runtime/src/egress/`
- Manifest v3 contract (v2 remains the legacy/resolved model):
  `crates/extensions/ironclaw_extension_registry/src/v3.rs`

## Quick implementation checklist

1. Pick the implementation: WASM (`[runtime] kind = "wasm"`), hosted MCP
   (`[mcp]`), or a channel surface (`[channel]`).
2. Create the package directory `crates/extensions/packages/<extension>/`
   with `manifest.toml`, `schemas/`, `prompts/`, and any `wasm/` module.
3. Write a `reborn.extension_manifest.v3` manifest with `[[tools]]` entries
   (each with an `origin_gate_matrix`), `[[tools.credentials]]`, and an
   `[auth.<vendor>]` recipe per credential vendor, and make it flow through
   extension registry discovery/publication.
4. Add schemas and prompt docs for every model-visible capability.
5. Implement runtime code using host services only.
6. Declare credentials with narrow HTTPS audiences and provider scopes.
7. Add packaging/onboarding only if host-bundled.
8. Add manifest, packaging, runtime, auth/approval, and integration tests.
9. Run targeted tests.
10. Check docs/specs and `FEATURE_PARITY.md` for behavior-status updates.
11. When your tool is packaged and tested, submit it to IronHub so other agents can discover and install it. See [Contributing](/hub/contributing).
