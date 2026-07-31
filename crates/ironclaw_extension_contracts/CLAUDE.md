# ironclaw_extension_contracts

The extension tier's neutral contract: what an installable extension **declares
and exposes**. Carved out of `ironclaw_host_api` by WS1.3 of the target
architecture (PROPOSAL §6.1.2, `docs/reborn/target-architecture/families/contracts.md`).

## What belongs here

A type is admitted iff all four hold (the contracts-family test, §6.1):

1. it names a concept crossing the host↔extension membrane;
2. it is neutral across vendor, runtime, storage, and deployment;
3. two or more consumers need it without importing an owner;
4. it carries no execution, persistence, policy engine, or workflow.

Today that is nine modules:

| Module | Owns |
| --- | --- |
| `channel` | Channel manifest-surface descriptors: `ChannelDescriptor`, `ChannelIngressDescriptor`, `ChannelEgressDescriptor`, `ChannelPresentation`, connection strategy/notices, and their validators. |
| `channel_identity` | The channel-identity hooks a host runs around binding: `ChannelConnectionScopeSource`, `ChannelIdentityPostBind(Factory)`, `ChannelIdentityOverride`. |
| `extension` | `Extension`, `ExtensionContract`, `ExtensionRuntimeIdentity`, `ExtensionInstanceId`, `ExtensionHostAssemblyConfig`. |
| `memory` | The `[memory]` manifest surface: `MemoryDescriptor`, `MemoryLifecycleHook`. |
| `package_lifecycle` | Package/extension lifecycle projection vocabulary (`Lifecycle*`, `ChannelConnectStrategy`, `ChannelConfigField`). |
| `preference_target` | `PreferenceTargetCodec` + `PreferenceTargetEncodeRequest` — the one vendor-implemented port here. |
| `recipe` | The auth recipe schema: `VendorAuthRecipe`, `OAuth2CodeRecipe`, `PkceMode`, ingress-verification recipes, and friends. |
| `state` | The installation state machine: `InstallationState`, `LifecyclePublicState`. |
| `surface` | `CapabilitySurfaceKind` — the manifest surface kinds an extension may declare. |

## What must never be here

The registry or installation stores (`ironclaw_extensions`); lifecycle
execution, binding orchestration, or ingress routing
(`ironclaw_extension_host`); product workflow; WASM/MCP mechanics; vendor names
(the specificity scanner polices this crate like any other); any implementation
of a port declared here (§6.1.4's rule applies family-wide — the
`PreferenceTargetCodec` implementations live in the Slack and Telegram packages,
which is the point).

## Dependencies

`ironclaw_host_api` and nothing else internal. No framework, driver, or runtime
client — no `axum`, `reqwest`, `wasmtime`, `libsql`, `tokio`. Validation
failures are reported as `ironclaw_host_api::error::HostApiError`; this crate
deliberately does **not** introduce a parallel error type for the same contract
failures.

## Admission tests

Three architecture tests hold the line, all runnable with
`cargo test -p ironclaw_architecture`:

- `reborn_dependency_boundaries.rs` — the §11.2.3 internal-dependency allowlist
  (`ironclaw_host_api` only, an allowlist so a future edge cannot slip past a
  list of today's offenders), the external framework/driver deny shared with the
  other contracts crates, and the crate's `BoundaryRule`.
- `reborn_extension_contract_location_scan.rs` — the §11.2.4 port-location rule:
  one definition per contract workspace-wide, and one import path (no crate
  re-exports one). Read its module doc before adding a `pub use` anywhere that
  names a type from here; the dual-path re-export is exactly the defect it
  exists to prevent, and the extension tier had three live instances of it.
- `reborn_extension_specificity.rs` — vendor-name scanning, which reaches this
  crate automatically through `cargo metadata`.

## Known interim placements

`package_lifecycle` sits here as a **forced co-mover**: PROPOSAL §6.1.3 assigns
it to `ironclaw_product_contracts`, but it is typed on `InstallationState`,
`LifecyclePublicState`, `ChannelPresentation`, and `CapabilitySurfaceKind` — all
§6.1.2 vocabulary — and `ironclaw_host_api` may hold no internal dependency, so
leaving it behind would have blocked those four from moving at all. Since
§6.1.3 lets `product_contracts` depend on this crate, WS1.4 can re-home it at
zero architectural cost; nothing here depends on it staying.

Deferred out of this crate for the same mechanical reason, in the other
direction — `ironclaw_host_api::product_surface` still names them, and it is
`product_contracts` material that cannot come here:

- `ChannelAdapter` and its DTO family (`host_api::product_adapter::channel_adapter`);
- `ToolAdapter` + `RestrictedEgress` (`host_api::tool_adapter`).

Both move once `product_surface` leaves `host_api` in WS1.4. Their conformance
suite (§11.2.10) follows them.
