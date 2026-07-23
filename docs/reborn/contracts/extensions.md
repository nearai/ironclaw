# Reborn Extensions Contract

**Status:** Draft implementation contract
**Date:** 2026-04-24
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/filesystem.md`, `crates/ironclaw_host_api`, `crates/ironclaw_filesystem`

---

## 1. Purpose

`ironclaw_extensions` owns extension package metadata, manifest validation, filesystem discovery, and capability declaration registration.
It also owns package manifests and caller-membership installation records.
Caller membership is the only installation-lifecycle authority; runtime
publication and administrator configuration are separate host concerns. Domain
crates such as `ironclaw_product_adapter_registry` project their own host API
sections from that generic state rather than owning a second installation
store.

It answers:

```text
What extension packages are installed?
What capabilities do they declare?
Which runtime lane should execute each capability?
What authority metadata do they request?
```

It does **not** execute capabilities.

Execution belongs to:

- `ironclaw_wasm` for WASM modules
- `ironclaw_scripts` for Docker-backed native CLI/script capabilities
- `ironclaw_mcp` for MCP adapter calls
- host-policy-selected service crates for first-party/system work

---

## 2. Core invariant

```text
ironclaw_extensions knows what can run.
runtime crates know how to run it.
```

`ExtensionManager` / `ExtensionRegistry` must not become a hidden runtime dispatcher. It may register descriptors and runtime metadata, but it must not load WASM, spawn Docker containers, connect to MCP servers, call network clients, resolve secrets, or spend budget.

---

## 3. Filesystem layout

V1 installed extensions live under:

```text
/system/extensions/<extension_id>/
```

Generic runtime and caller-membership state lives in filesystem record rows under:

```text
/system/extensions/.installations/manifests/<hashed_extension_id>.json
/system/extensions/.installations/installations/<hashed_installation_id>.json
```

The store declares exact indexes for extension id and runtime installation id,
and uses row CAS for membership, compatibility health metadata, and deletes.
Health metadata is diagnostic, not lifecycle authority. These records do not
own administrator configuration.

Manifest-declared administrator configuration is stored once per tenant under
the tenant-rewriting admin-configuration scoped mount, at the logical record
prefix `/extension-admin-configuration/groups`. Its stable key is the
`[admin_configuration].group_id`, so multiple manifests may consume one
identical group. Secrets are referenced by opaque handles and only presence is
projected. Admin configuration is deployment state: saving it does not add any
user to an extension's membership set.

Recommended package layout:

```text
/system/extensions/<extension_id>/
  manifest.toml
  SKILL.md
  skills/
  scripts/
  wasm/
  capabilities.json
  config/
  state/
  cache/
```

Rules:

- `<extension_id>` must match the manifest `id`.
- extension IDs use `ironclaw_host_api::ExtensionId` validation.
- manifest-local paths are relative package asset paths.
- manifest-local paths must not be absolute, scoped aliases, URLs, raw host paths, contain `..`, contain backslashes, or contain control characters.
- resolved assets become `VirtualPath`s under `/system/extensions/<extension_id>/...`.
- extension-local `config/`, `state/`, and `cache/` are package namespaces, not raw host paths.

---

## 4. Manifest schema

Production manifests use `schema_version = "reborn.extension_manifest.v2"`.
The older top-level `parameters_schema` manifest shape is no longer parsed on
production discovery paths.

Every manifest — host-bundled exactly as installed — declares its sections
through `[[host_api]]` contracts; tools live under
`[[host_api]] id = "ironclaw.capability_provider/v1"`. Top-level
`[[capabilities]]` is rejected for every manifest source.

V3 manifests may additionally declare one deployment-owned form:

```toml
[admin_configuration]
group_id = "vendor.example"
display_name = "Example deployment credentials"
description = "Credentials shared by Example extensions in this tenant."
fields = [
  { handle = "example_client_id", label = "Client ID", secret = false, required = true },
  { handle = "example_client_secret", label = "Client secret", secret = true, required = true },
]
```

This is the sole schema for operator-supplied extension configuration. It is
not nested below `[channel]`, is not copied into an installation record, and
may support channel, OAuth, MCP, tool, or future surfaces. Equal group ids must
carry equal descriptors.

Proof-code channel manifests may grant provider-specific inbound command
syntax declaratively:

```toml
[channel.connection]
strategy = "web_generated_code"
inbound_code_prefixes = ["/connect"]
```

`inbound_code_prefixes` contains at most eight unique, non-empty prefixes of at
most 32 bytes each. Prefixes cannot contain whitespace or control characters,
and are invalid for OAuth and administrator-managed strategies. The generic
pairing parser always accepts a bare proof code and strips only a
manifest-declared prefix followed by whitespace; it has no implicit provider
commands.

Host-bundled WASM manifest:

```toml
schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
```

Host-bundled script/CLI manifest:

```toml
schema_version = "reborn.extension_manifest.v2"
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
description = "Project-local CLI helpers"
trust = "untrusted"

[runtime]
kind = "script"
runner = "docker"
image = "python:3.12-slim"
command = "pytest"
args = ["tests/"]

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "project-tools.pytest"
description = "Run pytest"
effects = ["execute_code"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/project-tools/pytest.input.v1.json"
output_schema_ref = "schemas/project-tools/pytest.output.v1.json"
```

Host-bundled MCP adapter manifest:

```toml
schema_version = "reborn.extension_manifest.v2"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"
command = "github-mcp-server"
args = ["--stdio"]

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github-mcp/search_issues.input.v1.json"
output_schema_ref = "schemas/github-mcp/search_issues.output.v1.json"
prompt_doc_ref = "prompts/github-mcp/search_issues.md"
```

Host-mediated runtime credential manifest excerpt:

```toml
[[capability_provider.tools.capabilities]]
id = "github.search_issues"
description = "Search GitHub issues and pull requests."
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github/search_issues.input.v1.json"
output_schema_ref = "schemas/github/search_issues.output.v1.json"
prompt_doc_ref = "prompts/github/search_issues.md"
required_host_ports = ["host.runtime.http_egress"]
runtime_credentials = [
  { handle = "github_runtime_token", source = { type = "product_auth_account", provider = "github" }, audience = { scheme = "https", host_pattern = "api.github.com" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]
```

### Host API contracts

V2 keeps one extension identity and lets that extension implement one or more host API contracts. `host_api.id` is the only top-level contract/type discriminator; there is no separate manifest `kind`.

```toml
schema_version = "reborn.extension_manifest.v2"
id = "telegram"
name = "Telegram"
version = "0.1.0"
description = "Telegram product adapter and tools"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/telegram.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[product_adapter.inbound]
surface_kind = "telegram"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "telegram.send_message"
description = "Send a Telegram message to a chat."
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/telegram/send_message.input.v1.json"
output_schema_ref = "schemas/telegram/send_message.output.v1.json"
prompt_doc_ref = "prompts/telegram/send_message.md"
```

Rules:

- `ironclaw_extensions` parses the envelope, validates host API refs, and dispatches to a composition-wired host API contract registry.
- Domain contract handlers own section pattern validation, cardinality, typed section schema validation, and catalog/read-model projection.
- Domain contract handlers must not treat manifest `trust` / `descriptor_trust_default` as effective runtime authority. Effective trust and grants come from composition-owned trust policy evaluation, not self-declared manifest metadata.
- Model-visible capability-provider sections must carry enough cold metadata to project an LLM-facing tool descriptor: stable capability ID, human description, input schema ref, output schema ref, effects, permission default, and visibility. `prompt_doc_ref` is optional lazy help metadata, not part of the mandatory per-turn surface.
- The LLM consumes the projected hot capability surface, not the raw manifest section. Catalog publication resolves schema refs into compact per-turn tool descriptors and resolves `prompt_doc_ref` only when one is declared.
- Unknown `host_api.id` values fail closed.
- Repeating the same `host_api.id` is allowed only when that contract declares multi-instance support.
- Every `[[host_api]]` must reference an existing explicit `section` path.
- Operational sections must be referenced by `[[host_api]]`; inert metadata may live under `[metadata.*]` or `[x.*]`.
- Manifest validation is atomic: any invalid host API contract invalidates the extension manifest.
- Runtime loading, handshakes, catalog publication, authority grants, and execution remain outside `ironclaw_extensions`.

Cutover (complete):

- Every manifest — host-bundled exactly as installed — declares at least one
  `[[host_api]]` contract; there is no contract-free manifest form and no
  contract-free parse or discovery entry point.
- Top-level `[[capabilities]]` is rejected for every manifest source with an
  actionable error. Capabilities are declared under
  `[[capability_provider.tools.capabilities]]` and referenced from
  `[[host_api]] id = "ironclaw.capability_provider/v1"`.
- Host API contracts raise `HostApiSectionError`: this crate's own contracts
  (capability provider) preserve typed `ManifestV2Error` variants; domain
  crates report redacted reasons wrapped as `HostApiSectionRejected`.

### Capability surfaces

The extension is the top-level product object; a *capability surface* is one
product-facing face the manifest declares. `CapabilitySurfaceKind`
(`ironclaw_host_api`) enumerates the vocabulary: `tool`, `channel`, `auth`,
plus reserved `trigger` and `file`. The host discovers and wires generic
services from declared surfaces; it must not maintain a separate first-class
channel registry beside the extension registry, and runtime kind (`wasm` /
`mcp` / `first_party`) must never decide surface taxonomy.

Rules:

- Surfaces are **derived** vocabulary. The owning manifest declarations stay
  the single source of truth; `ExtensionManifestV2::capability_surfaces()`
  projects them on demand:
  - each capability declaration projects one `tool` surface;
  - each host API contract section projects the surface kinds its contract
    declares via `HostApiManifestProjection::surfaces` (the
    `ironclaw.product_adapter/v1` contract projects `channel` for
    `external_channel` sections; host-native product surface kinds — `web`,
    `cli`, `synchronous_api` — project nothing), origin-stamped with the
    owning host API id and section path;
  - `product_auth_account` runtime-credential sources project one `auth`
    surface per distinct provider id. OAuth setups fold to the union of
    declared scopes (sorted, deduplicated) and mask weaker manual-token
    setups; a provider referenced only through retired setups surfaces as
    retired rather than being dropped.
- Host API contracts must not project `tool` or `auth` section surfaces —
  those kinds have dedicated declaration paths above. Validation fails
  closed.
- `VendorId` is the credential
  authority namespace, not the extension id: several extensions (gmail,
  google-drive, ...) may share one provider (`google`).

Tests: `crates/ironclaw_extensions/tests/manifest_v2_contract.rs`
(capability surface projection block) and
`crates/ironclaw_product_adapter_registry/tests/manifest_ingestion.rs`
(channel-surface projection through the real product-adapter contract). Run:
`cargo test -p ironclaw_extensions --test manifest_v2_contract` and
`cargo test -p ironclaw_product_adapter_registry --test manifest_ingestion`.

---

## 5. Runtime declarations

Manifest runtime kinds map to `ironclaw_host_api::RuntimeKind`:

| Manifest `kind` | RuntimeKind | Meaning |
|---|---|---|
| `wasm` | `Wasm` | portable module lane |
| `script` | `Script` | native CLI/script lane selected by a semantic runner profile |
| `mcp` | `Mcp` | MCP adapter lane |
| `first_party` | `FirstParty` | host-policy-selected packaged service/loop ceiling; not authority by itself |
| `system` | `System` | host-owned system fixture/service only; not user-installable |

Runtime metadata is declarative. It is passed to the appropriate runtime crate later.

Rules:

- WASM declarations may name module assets but must not load modules.
- Script declarations may name a semantic runner, command, args, and optional backend-specific image metadata, but must not execute or expose raw Docker flags.
- MCP declarations may describe stdio/remote transport but must not connect during manifest parsing/registry insertion.
- Host/system declarations require matching trust ceilings and should be rare, host-policy-assigned, and never self-declared by ordinary user-installed packages.
- Runtime/trust declarations are not grants; privileged effects still require capability grants, mounts, leases, obligations, and resource policy.
- Extension or loop upgrades that change package identity, signer/source policy, trust class, or requested authority require renewed approval/admin policy before old grants apply.

---

## 6. Capability declarations

Each capability declaration produces a `CapabilityDescriptor`.

Rules:

- capability ID must be valid `CapabilityId`.
- capability ID must be prefixed by the provider extension ID: `<extension_id>.<name>`.
- descriptor `provider` is always the manifest extension ID.
- descriptor `runtime` is inherited from the manifest runtime declaration unless a future schema explicitly allows per-capability runtime overrides.
- descriptor `trust_ceiling` comes from the manifest's safe
  `descriptor_trust_default`, not from effective runtime trust.
- effects must parse as `EffectKind`.
- default permission must parse as `PermissionMode`.
- `runtime_credentials` declares host-owned credential injection metadata for
  runtime HTTP egress. Each entry names a runtime credential slot handle,
  material source (`secret_handle` by default, or `product_auth_account` with a
  provider id), HTTPS-only audience `NetworkTargetPattern`, injection target
  (`header` or `query_param`), and optional `required` flag. The field is only
  valid when the capability declares `use_secret`; duplicate handles within one
  capability are invalid. The manifest never contains raw secret material.
- every capability must provide `input_schema_ref` and `output_schema_ref`;
  `prompt_doc_ref` is optional lazy help metadata.
- during this cutover, `CapabilityDescriptor.parameters_schema` is a projection
  placeholder of the form `{ "$ref": input_schema_ref }`. Catalog publication is
  responsible for resolving schema/doc refs into hot per-turn tool descriptors.

---

## 7. Registry contract

`ExtensionRegistry` owns validated descriptors.

Rules:

- duplicate extension ID is rejected.
- duplicate capability ID across extensions is rejected.
- registry insertion validates descriptor/provider consistency.
- lookup by extension ID returns package metadata and runtime declaration.
- lookup by capability ID returns the descriptor and provider package.
- registry does not execute, authorize, or reserve resources.

---

## 8. Discovery contract

`ExtensionDiscovery` reads from the filesystem service, not raw host paths.

Flow:

```text
RootFilesystem.list_dir(/system/extensions)
  -> for each child directory
  -> read /system/extensions/<extension>/manifest.toml
  -> parse and validate manifest
  -> verify manifest id matches directory id
  -> register package/descriptors
```

Rules:

- missing root fails clearly.
- missing manifest fails clearly.
- malformed manifest fails clearly.
- invalid IDs fail closed.
- discovered packages are deterministic, preferably sorted by extension ID.
- discovery does not load runtime artifacts or connect to external services.

---

## 9. Error contract

Minimum errors:

```rust
ExtensionError::ManifestParse
ExtensionError::InvalidManifest
ExtensionError::InvalidAssetPath
ExtensionError::ManifestIdMismatch
ExtensionError::DuplicateExtension
ExtensionError::DuplicateCapability
ExtensionError::Filesystem
```

Errors should reference virtual paths or extension IDs, not raw host paths.

---

## 10. Initial Rust API sketch

```rust
pub struct ExtensionPackage {
    pub id: ExtensionId,
    pub root: VirtualPath,
    pub manifest: ExtensionManifest,
    pub capabilities: Vec<CapabilityDescriptor>,
}

pub enum ExtensionRuntime {
    Wasm { module: ExtensionAssetPath },
    Script { runner: String, image: Option<String>, command: String, args: Vec<String> },
    Mcp { transport: McpTransport, command: Option<String>, args: Vec<String>, url: Option<String> },
    FirstParty { service: String },
    System { service: String },
}

pub struct ExtensionRegistry {
    pub fn insert(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError>;
    pub fn get_extension(&self, id: &ExtensionId) -> Option<&ExtensionPackage>;
    pub fn get_capability(&self, id: &CapabilityId) -> Option<&CapabilityDescriptor>;
}

pub struct ExtensionDiscovery;

impl ExtensionDiscovery {
    pub async fn discover<F: RootFilesystem>(
        fs: &F,
        root: &VirtualPath,
    ) -> Result<ExtensionRegistry, ExtensionError>;
}
```

---

## 11. Minimum TDD coverage

Local contract tests should prove:

- valid WASM manifest parses and extracts `CapabilityDescriptor`.
- invalid extension ID is rejected.
- capability ID must be provider-prefixed.
- runtime kind and trust ceiling parse correctly.
- script runtime declaration stores semantic runner metadata without executing it; legacy Docker metadata remains accepted only for the optional Docker backend.
- MCP runtime declaration stores transport metadata without connecting.
- invalid manifest-local asset paths are rejected.
- registry rejects duplicate extension IDs.
- registry rejects duplicate capability IDs.
- discovery reads manifests via `RootFilesystem` and `/system/extensions` virtual paths.
- discovery rejects missing manifest.
- discovery rejects manifest ID mismatch with directory name.
- capability-surface projection: tool-only, channel-only, and tool+channel
  manifests project exactly their declared surfaces; auth surfaces group by
  provider with unioned OAuth scopes; extensions sharing one provider project
  the same provider id (distinct from their extension ids); contracts
  projecting `tool`/`auth` section surfaces fail closed.

---

## 12. Non-goals

Do not add in `ironclaw_extensions` V1:

- WASM module loading
- Docker/container execution
- MCP client connections
- network calls
- resource reservation enforcement
- secret resolution
- marketplace install flows
- OAuth/authentication
- product workflows
- agent loop behavior


---

## Contract freeze addendum — lifecycle scope (2026-04-25)

The V1 extension contract freezes the full lifecycle even if implementation lands in slices:

```text
discover
install
authenticate
configure
activate
deactivate
remove
upgrade
failed/retry
```

The extension registry/package source of truth is typed extension state with optional `/system/extensions/...` file projections. Extension config/state projections must validate through the typed repository and must not bypass lifecycle authorization.

WASM, Script, and MCP are all first-class v2 runtime lanes; extension manifests and lifecycle state must be able to describe each lane without making dispatcher depend on concrete runtime crates.

### Current product projection

The dated freeze above records the original internal lifecycle scope. It does
not define the current product-facing state machine or authorize public
Activate/Deactivate operations. Current APIs expose one derived projection:

```text
caller is not a member                          -> uninstalled
member has any non-ready typed readiness result -> setup_needed
member has a complete typed readiness result    -> active
```

Install joins the authenticated caller to membership and immediately runs
generic readiness reconciliation. Remove removes membership; it is the only
user-facing way to disable an extension. Internal loading, discovery,
provisioning, atomic publication, cleanup, and upgrade checkpoints may retain
implementation names such as `activate` or `deactivate`, but they must never
surface as additional states or required user actions.

The host's complete typed readiness result includes every manifest-declared
tenant and personal prerequisite plus bind, discovery, provisioning, conflict,
and atomic-publication outcomes. A first publication that cannot complete has
no callable surface. A refresh is different: once a catalog is active, a
failed refresh reports its own error but retains the last successfully
published callable surface and therefore the caller's `active` projection.
Refreshing never pre-emptively demotes or unpublishes a working generation.

Tenant `[admin_configuration]` and caller lifecycle are independent: an admin
may configure a group before any user installs its consuming extensions, and
each user's membership, OAuth grants, and channel pairings remain isolated.
