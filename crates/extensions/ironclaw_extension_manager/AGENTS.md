# `ironclaw_extension_manager` — the product face of extensions

Split out of `ironclaw_extension_host` in WS2.4 (PROPOSAL §6.8.3). The host
owns lifecycle **authority**; this crate owns extension-management **UX
semantics** on top of it.

## The one rule that makes the crate worth existing

**The manager calls the host. The host never calls the manager.**

`ironclaw_extension_host` has moved from the `products` layer down to `loops`
(§12.1c; its manifest now declares `layer = "loops"`). It could not while it
held product-facing code, because `products` sits above `loops` and every
product symbol would have become an upward dependency. Splitting that code out
is what unblocked the flip — and the flip stays legal only while the edge
stays one-way. A back-edge of **any** kind, including a `[dev-dependencies]`
entry for a test fixture, puts the two crates in a cycle the layer matrix
cannot satisfy.

`crates/app/ironclaw_architecture_tests/tests/reborn_extension_manager_split.rs` enforces
this at the manifest *and* the source level. If you need host code to reach
something here, the answer is to move that code here or to invert it behind a
port the host declares — never to add the dependency.

## What lives here

| Module | Surface |
|---|---|
| `extension_lifecycle_capabilities` | the agent-callable lifecycle capabilities (`extension_install` / `activate` / `remove` / `search` …) |
| `extension_lifecycle_command` | the `ironclaw extension` CLI command and its rendering |
| `lifecycle_product_service` | `ExtensionHostLifecycleProductService` — the `LifecycleProductService` port the WebUI routes through |
| `channel_config_product_service` | `RebornChannelConfigProductService` — the product projection over the host's `ChannelConfigService` |
| `admin_configuration` | the administrator-configuration `RebornViewProvider` |
| `admin_configuration_capability` | the administrator-configuration replace capability |
| `operator_config_capability` | operator auto-approve / tool-permission capabilities |
| `skill_auto_activate_capability` | the skill auto-activate capability |
| `webui_extension_credentials` | `ProductAuthExtensionCredentialSetup` — the credential setup/status views |
| `ironhub` | the extension hub: search / info / install, as capabilities and as a CLI command |
| `lifecycle_test_support` (feature `test-support`) | the full lifecycle service bundle downstream integration tests drive |

## What deliberately stayed in the host

Not everything the §6.8.3 inventory names could move, and the reasons are
structural rather than effort:

- **`ExtensionLifecycleManager` (`product_lifecycle.rs`)** is the lifecycle
  *workflow* — it holds the operation lock, drives activation transactions, and
  is what §6.8.3 means by "calls `extension_host`". It is also mutually
  recursive with the hosted-MCP registration pipeline (§6.8.2's amendment), so
  it cannot cross a crate boundary in either direction without an inversion.
- **The available-extension catalog** (`available_extensions.rs`,
  `available_extension_import.rs`) is read by the host's own
  `lifecycle_restore` at boot and by the registration pipeline. It is
  infrastructure the host needs, not only a product view.
- **The `ChannelConfigService` and `ChannelPairingService` cores.** §6.8.2
  keeps both; five and seven host modules respectively consume them. Only the
  product *projection* over channel config moved.
- **`admin_configuration_service` / `admin_configuration_store`** — consumed by
  `channel_host.rs` and `channel_subject_routes.rs`. §6.8.3 assigns the
  *capability handlers* to the manager, not the store.

## The `ironclaw_assistant` dependency is residue, not the design

`docs/reborn/target-architecture/families/extensions.md` gives this crate
`product_contracts` + `extension_contracts` + `extension_registry` +
`extension_host`. It also depends on `ironclaw_assistant` today, in exactly seven
files, and every one is a **product DTO, a capability-id constant, or one of
two port-inversion residues** (the `ExtensionCredentialSetupService` port and
the auth-continuation fixture wiring) — never a workflow call. Those symbols
belong in `ironclaw_product_contracts` by §6.1.3 and are waiting on rows that
own them.

The list is frozen exact-match and shrink-only in
`reborn_extension_manager_split.rs`. Adding a file to it is not how you make a
change compile: move the symbol to `ironclaw_product_contracts` instead.

## Conventions

- Import host types from `ironclaw_extension_host` directly; this crate
  re-exports only its own two headline types.
- Never write installation state here. Every mutation goes through
  `ironclaw_extension_host::ExtensionLifecycleManager`.
- `test_support/lifecycle.rs` keeps that path rather than a flat
  `lifecycle_test_support.rs`: the extension-specificity scanner treats a
  `test_support` *path component* as fixture code, where naming a concrete
  vendor is legal (overview §8). Flattening it would push a fixture into the
  generic-code scan.
