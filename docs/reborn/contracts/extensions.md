# Reborn Extensions Contract

**Status:** Draft implementation contract
**Date:** 2026-04-24
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/filesystem.md`, `crates/ironclaw_host_api`, `crates/ironclaw_filesystem`

---

## 1. Purpose

`ironclaw_extensions` owns extension package metadata, manifest validation, filesystem discovery, and capability declaration registration.

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

Minimal V1 manifest:

```toml
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
```

Script/CLI manifest example:

```toml
id = "project-tools"
name = "Project Tools"
version = "0.1.0"
description = "Project-local CLI helpers"
trust = "sandbox"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "pytest"
args = ["tests/"]

[[capabilities]]
id = "project-tools.pytest"
description = "Run pytest"
effects = ["execute_code", "read_filesystem", "write_filesystem"]
default_permission = "ask"
parameters_schema = { type = "object" }
```

MCP adapter manifest example:

```toml
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "user_trusted"

[runtime]
kind = "mcp"
transport = "stdio"
command = "github-mcp-server"
args = ["--stdio"]

[[capabilities]]
id = "github-mcp.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
```

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
- descriptor `trust_ceiling` is inherited from manifest `trust`.
- effects must parse as `EffectKind`.
- default permission must parse as `PermissionMode`.
- missing schema defaults to `{ "type": "object" }` only if explicitly chosen by implementation; otherwise missing schema should fail in V1.

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

WASM, Script, and MCP are all first-class V1 runtime lanes; extension manifests and lifecycle state must be able to describe each lane without making dispatcher depend on concrete runtime crates.
