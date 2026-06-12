# Add IronHub as a catalog source for Reborn extensions

## Problem

IronHub is the v1 tool/channel registry located at `registry/tools/` in the repo root. Each tool has a JSON manifest (e.g. `registry/tools/github.json`) and a sidecar capabilities JSON file (e.g. `tools-src/github/github-tool.capabilities.json`). These manifests describe WASM artifacts hosted at remote URLs with SHA-256 checksums.

The Reborn extension system (`AvailableExtensionCatalog` in `crates/ironclaw_reborn_composition/src/available_extensions.rs`) currently has two sources for discoverable extensions:

1. **First-party bundled extensions** — loaded via `from_first_party_assets_with_nearai_mcp_config()` (available_extensions.rs:265). These use `include_str!`/`include_bytes!` to embed TOML manifests and WASM modules at compile time.
2. **Filesystem-installed extensions** — loaded via `from_filesystem_root()` (available_extensions.rs:297). These scan `system/extensions/` on the local filesystem for pre-installed TOML manifests.

IronHub tools in `registry/tools/*.json` are **invisible** to `extension search`, `extension install`, and `extension activate`. There is no code path that reads the IronHub JSON format, synthesizes a Reborn extension manifest, or downloads remote WASM artifacts.

## Proposed Solution

Add a third catalog source: `from_ironhub_registry(tools_dir: &Path)` on `AvailableExtensionCatalog`. This method reads IronHub JSON manifests from the filesystem, pairs them with their sidecar capabilities JSON, synthesizes Reborn extension manifests in memory, and returns `AvailableExtensionPackage` entries.

For the install flow, detect that an extension originated from IronHub (its WASM artifact is at a remote URL rather than bundled or local) and download + verify the artifact before materializing.

## Implementation Details

### 1. `crates/ironclaw_reborn_composition/src/available_extensions.rs`

#### a. Add `IronHubManifest` struct for JSON deserialization

```rust
#[derive(Deserialize)]
struct IronHubManifest {
    name: String,
    display_name: String,
    kind: String,
    version: String,
    description: String,
    source: IronHubSource,
    artifacts: IronHubArtifacts,
    auth_summary: Option<IronHubAuthSummary>,
}

#[derive(Deserialize)]
struct IronHubSource {
    dir: String,
    capabilities: String,
}

#[derive(Deserialize)]
struct IronHubArtifacts {
    #[serde(rename = "wasm32-wasip2")]
    wasm32_wasip2: Option<IronHubArtifact>,
}

#[derive(Deserialize)]
struct IronHubArtifact {
    url: String,
    sha256: String,
}

#[derive(Deserialize)]
struct IronHubAuthSummary {
    method: Option<String>,
    secrets: Option<Vec<String>>,
}
```

#### b. Add capabilities JSON sidecar structs

```rust
#[derive(Deserialize)]
struct IronHubCapabilities {
    capabilities: Option<IronHubCapabilitiesBody>,
}

#[derive(Deserialize)]
struct IronHubCapabilitiesBody {
    http: Option<IronHubHttpCapability>,
    secrets: Option<IronHubSecretsCapability>,
}

#[derive(Deserialize)]
struct IronHubHttpCapability {
    allowlist: Option<Vec<IronHubHttpAllowlistEntry>>,
}

#[derive(Deserialize)]
struct IronHubHttpAllowlistEntry {
    host: String,
}

#[derive(Deserialize)]
struct IronHubSecretsCapability {
    allowed_names: Option<Vec<String>>,
}
```

#### c. Add `from_ironhub_registry()` to `AvailableExtensionCatalog`

Pseudo-logic:

```
def from_ironhub_registry(tools_dir):
    if !tools_dir.exists():
        return Ok(Self::empty())

    packages = []
    for entry in glob(tools_dir/*.json):
        manifest_json = read_file(entry)
        ironhub_manifest: IronHubManifest = serde_json::from_str(manifest_json)

        if ironhub_manifest.kind != "tool":
            continue  # channel manifests not yet supported

        // Resolve sidecar capabilities path relative to repo root
        let caps_path = tools_dir.join("..").join("..").join(ironhub_manifest.source.dir)
            .join(ironhub_manifest.source.capabilities);
        let caps_json = read_file(caps_path);
        let caps: IronHubCapabilities = serde_json::from_str(caps_json);

        let package = convert_ironhub_tool(ironhub_manifest, caps)?;
        packages.push(package);

    Ok(Self::from_packages(packages))
```

#### d. Add `convert_ironhub_tool()` helper

Synthesizes a TOML manifest string:

```
schema_version = "reborn.extension_manifest.v2"
id = "{manifest.name}"
name = "{manifest.display_name}"
version = "{manifest.version}"
description = "{manifest.description}"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/{manifest.name}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{manifest.name}.invoke"
description = "{manifest.description}"
effects = {derived from caps}
default_permission = "ask"
visibility = "model"
required_host_ports = ["host.runtime.http_egress"]
```

Effects derivation from capabilities JSON:
- If `caps.http.allowlist` is non-empty → include `"network"` in effects
- If `caps.secrets.allowed_names` is non-empty → include `"use_secret"` in effects

If `manifest.auth_summary.method == "oauth"`, add `runtime_credentials` to the synthesized capability block.

#### e. Modify `materialize_available_extension()` to handle remote downloads

Add a third `AvailableExtensionAssetContent` variant:

```rust
pub(crate) enum AvailableExtensionAssetContent {
    Bytes(Vec<u8>),
    Filesystem(VirtualPath),
    Remote {
        url: String,
        sha256: String,
    },
}
```

When encountering `Remote { url, sha256 }` in the materialization loop:
1. Download the WASM from `url` via HTTP(S)
2. Compute SHA-256 of the downloaded bytes
3. Verify it matches `sha256`; abort with error on mismatch
4. Write the verified bytes to the target path

### 2. `crates/ironclaw_reborn_composition/src/input.rs`

Add to `RebornBuildInput`:

```rust
pub(crate) ironhub_tools_dir: Option<PathBuf>,
```

Builder method:

```rust
pub fn with_ironhub_tools_dir(mut self, tools_dir: PathBuf) -> Self {
    self.ironhub_tools_dir = Some(tools_dir);
    self
}
```

### 3. `crates/ironclaw_reborn_composition/src/factory.rs`

Destructure `ironhub_tools_dir` from `RebornBuildInput` in `build_local_dev()`.

After loading first-party and filesystem catalogs:

```rust
if let Some(tools_dir) = ironhub_tools_dir {
    let hub_extensions = AvailableExtensionCatalog::from_ironhub_registry(&tools_dir)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("IronHub tools catalog could not be loaded: {error}"),
        })?;
    available_extensions.extend(hub_extensions);
}
```

### 4. `crates/ironclaw_reborn_cli/src/runtime/mod.rs`

Auto-detect `registry/tools/` in `build_services_input_with_options()`:

```rust
let cwd = std::env::current_dir()
    .context("failed to resolve current directory")?;
let ironhub_tools_dir = cwd.join("registry").join("tools");
if ironhub_tools_dir.exists() {
    services_input = services_input.with_ironhub_tools_dir(ironhub_tools_dir);
}
```

## Edge Cases

| Edge case | Handling |
|-----------|----------|
| `registry/tools/` directory does not exist | Silently return empty catalog from `from_ironhub_registry()` — no error |
| No sidecar capabilities JSON file | Default to `effects = []` (safe but limited) |
| `auth_summary.method = "oauth"` | Synthesize `runtime_credentials` with `product_auth_account` source |
| No WASM artifact URL | Fail during `convert_ironhub_tool()` with clear error |
| Checksum verification failure | Abort `materialize_available_extension()` with mismatch details |
| HTTP download failure | Propagate as `ProductWorkflowError::Transient` |
| Manifest `kind != "tool"` | Skip silently (channel/MCP manifests out of scope) |
| Malformed JSON manifest | Return `ProductWorkflowError::InvalidBindingRequest` |
| Duplicate extension ID | IronHub takes precedence via `extend()` merge semantics |

## Acceptance Criteria

- [ ] `extension search` finds IronHub tools by name/description
- [ ] `extension install` downloads WASM artifact, verifies SHA-256, materializes
- [ ] `extension activate` works after successful install
- [ ] Running from a directory without `registry/tools/` shows no IronHub entries
- [ ] Existing first-party extensions still work unchanged
- [ ] Unit tests for `from_ironhub_registry()` and `convert_ironhub_tool()`

## Files to Modify

| File | Changes |
|------|---------|
| `crates/ironclaw_reborn_composition/src/available_extensions.rs` | Add IronHub structs, `from_ironhub_registry()`, `convert_ironhub_tool()`, `download_ironhub_artifact()`, extend `AvailableExtensionAssetContent` |
| `crates/ironclaw_reborn_composition/src/input.rs` | Add `ironhub_tools_dir` field + builder |
| `crates/ironclaw_reborn_composition/src/factory.rs` | Load IronHub catalog, extend available extensions |
| `crates/ironclaw_reborn_cli/src/runtime/mod.rs` | Auto-detect `cwd/registry/tools/` |

## Future Work

- Publishing new tools from Reborn to IronHub
- Channel manifests (`kind: "channel"`)
- MCP server manifests (`kind: "mcp_server"`)
- Skills from IronHub (SKILL.md format, not JSON)
- WASM caching by SHA-256
