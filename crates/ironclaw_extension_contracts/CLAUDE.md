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

Today that is thirteen modules:

| Module | Owns |
| --- | --- |
| `auth_prompt` | The channel-rendered auth challenge family: `AuthPromptView`, `AuthPromptChallengeKind`, `ConnectionPromptContext`, `PairingPromptView`, `AuthPromptContextView`, and `render_channel_auth_prompt`. Arrived with WS1.4 — `OutboundPart::AuthPrompt` names it and both channel packages render it. |
| `channel` | Channel manifest-surface descriptors: `ChannelDescriptor`, `ChannelIngressDescriptor`, `ChannelEgressDescriptor`, `ChannelPresentation`, connection strategy/notices, and their validators. |
| `channel_adapter` | `ChannelAdapter` + its DTO family (`VerifiedInbound`, `InboundOutcome`, `NormalizedInboundMessage`, `OutboundEnvelope`/`Part`/`Target`, `DeliveryReport`, `TargetQuery`/`Candidate`, `ChannelError`, `ChannelAttachmentRef`, `ProductTriggerReason`) — arrived with WS1.4. |
| `channel_identity` | The channel-identity hooks a host runs around binding: `ChannelConnectionScopeSource`, `ChannelIdentityPostBind(Factory)`, `ChannelIdentityOverride`. |
| `egress` | Channel egress transport vocabulary: `ProtocolHttpEgress`, the `Egress*` request/response types, `DeliveryStatus`/`OutboundDeliverySink`, `DeclaredEgressHost`/`Target` — arrived with WS1.4. |
| `extension` | `Extension`, `ExtensionContract`, `ExtensionRuntimeIdentity`, `ExtensionInstanceId`, `ExtensionHostAssemblyConfig`. |
| `external` | Vendor-side refs the adapter cone names: `ExternalActorRef`, `ExternalConversationRef`, `ExternalEventId`, `ProductAttachmentDescriptor`/`Kind` — arrived with WS1.4. |
| `memory` | The `[memory]` manifest surface: `MemoryDescriptor`, `MemoryLifecycleHook`. |
| `preference_target` | `PreferenceTargetCodec` + `PreferenceTargetEncodeRequest` — the one vendor-implemented port here. |
| `recipe` | The auth recipe schema: `VendorAuthRecipe`, `OAuth2CodeRecipe`, `PkceMode`, ingress-verification recipes, and friends. |
| `state` | The installation state machine: `InstallationState`, `LifecyclePublicState`. |
| `surface` | `CapabilitySurfaceKind` — the manifest surface kinds an extension may declare. |
| `tool_adapter` | `ToolAdapter` + `RestrictedEgress` and their call/result/error vocabulary — arrived with WS1.4. |
| `test_support` | Feature-gated: the exported channel-adapter conformance suite (§11.2.10) and the in-memory egress/delivery fakes. |

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

## Resolved placements (WS1.4)

`package_lifecycle` **left** for `ironclaw_product_contracts`, which is where
PROPOSAL §6.1.3 assigns it; WS1.3 had it here only as a forced co-mover and said
so. Nothing here depended on it staying.

`ChannelAdapter`, `ToolAdapter`, and `RestrictedEgress` **arrived**, unblocked
exactly as WS1.3 predicted: `host_api::product_surface` moved to
`ironclaw_product_contracts`, so nothing that stays in `ironclaw_host_api` names
them any more. `egress` and `external` came with them (§6.1.2 names both in its
"fed by" list, and the adapter cone types every one of them).

`auth_prompt` arrived for a reason §6.1.3's prose did not anticipate: it lists
"auth/approval prompt-view DTOs" as product-tier, but `ChannelAdapter`'s own
`OutboundPart::AuthPrompt` carries `AuthPromptView`, and both shipped channel
packages call `render_channel_auth_prompt` from `deliver`. The approval half
stayed in `product_contracts::outbound`. The module doc records the one
deliberate consequence: two ~15-line display-text validators exist in both
crates rather than making a generic validator part of this crate's public API.
