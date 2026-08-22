# ironclaw_extension_contracts — working rules

Canonical crate guidance (the crate's `CLAUDE.md` is a pointer here).
Orientation and public surface: [`README.md`](./README.md). Family boundary
and admission test: [`../AGENTS.md`](../AGENTS.md). Carved out of
`ironclaw_host_api` by WS1.3 of the target architecture (PROPOSAL §6.1.2,
`docs/internal/reborn/target-architecture/families/contracts.md`).

## What belongs here

A type is admitted iff all four parts of the contracts-family test hold
(family `AGENTS.md`): it names a concept crossing the host↔extension membrane;
it is neutral across vendor, runtime, storage, and deployment; two or more
consumers need it without importing an owner; it carries no execution,
persistence, policy engine, or workflow.

Re-derive the shipped-module count with
`rg -n "^pub mod " crates/contracts/ironclaw_extension_contracts/src/lib.rs | wc -l`,
minus one for the feature-gated `test_support` module (no architecture test
currently pins this count against the table below — treat the table as a
map, not an enforced inventory, until one exists):

| Module | Owns |
| --- | --- |
| `auth_prompt` | The channel-rendered auth challenge family: `AuthPromptView`, `AuthPromptChallengeKind`, `ConnectionPromptContext`, `PairingPromptView`, `AuthPromptContextView`, and `render_channel_auth_prompt`. Arrived with WS1.4 — `OutboundPart::AuthPrompt` names it and both channel packages render it. |
| `channel` | Channel manifest-surface descriptors: `ChannelDescriptor`, `ChannelIngressDescriptor`, `ChannelEgressDescriptor`, `ChannelPresentation`, connection strategy/notices, and their validators. |
| `channel_adapter` | The three optional channel capability traits — `ChannelIngress`, `ChannelReply`, and `ChannelDelivery` — plus `ChannelSurfaces` and their DTO family (`VerifiedInbound`, `InboundOutcome`, `NormalizedInboundMessage`/`InboundAttachment`, `OutboundEnvelope`/`Part`/`Target`, `DeliveryReport`, `DirectTargetProvisionRequest`, `ChannelError`, `ProductTriggerReason`). `ChannelAttachmentRef` is package-internal parse→fetch state, never host ingress vocabulary. The traits pair with vendor `[channel.ingress]`, message `[channel.reply]`, and `[channel.delivery]`; authenticated-session ingress and stream replies are host-owned and intentionally bind no adapter half. Direct-target provisioning is an optional typed operation on `ChannelDelivery`, not a target-search grammar. |
| `channel_identity` | The channel-identity hooks a host runs around binding: `ChannelConnectionScopeSource`, `ChannelIdentityPostBind(Factory)`, `ChannelIdentityOverride`. |
| `device_link` | The device-link auth hook: `DeviceLinkAdapter`, `DeviceLinkStep`, `DeviceLinkInput`, `DeviceLinkError`. The one auth method whose mechanics an extension implements instead of declaring; `DeviceLinkInput` deliberately does not implement `Serialize` so a step or event physically cannot carry a secret. |
| `egress` | Channel egress transport vocabulary: `ProtocolHttpEgress`, the `Egress*` request/response types, `DeliveryStatus`/`OutboundDeliverySink`, `DeclaredEgressHost`/`Target` — arrived with WS1.4. |
| `extension` | `Extension`, `ExtensionContract`, `ExtensionRuntimeIdentity`, `ExtensionInstanceId`, `ExtensionHostAssemblyConfig`. |
| `external` | Vendor-side refs the adapter cone names: `ExternalActorRef`, `ExternalActorBindingEpoch`, `ExternalConversationRef`, `ExternalEventId`, `ProductAttachmentDescriptor`/`Kind` — arrived with WS1.4; `ExternalActorBindingEpoch` joined with WS2.5's binding-epoch move. |
| `hosted_mcp` | Untrusted registration input for user-registered hosted MCP servers: `RegisterHostedMcpRequest`, `HostedMcpEndpoint`, `HostedMcpAuthSelection`, `McpAuthChallenge`, the auth-metadata extraction helper, and the WS3 discovery pair (`HostedMcpDiscoveredTool*`). |
| `lifecycle_id` | The bounded package-identity newtypes both tiers need: `LifecyclePackageId` (which `hosted_mcp` names structurally) and `LifecycleBlockerRef`. |
| `linked_session` | Linked-account session custody port: the extension-package-facing trait to load and compare-and-swap the one opaque credential blob a linked device session persists. Implemented only by the extension host; the package never names the auth domain directly. |
| `memory` | The `[memory]` manifest surface: `MemoryDescriptor`, `MemoryLifecycleHook`, and the `[[memory.scheduled_ops]]` declaration family (`MemoryScheduledOp`, `MemoryScheduledTrigger`, `MemoryScheduledOpKind`, `MemoryScheduledPass`, `MIN_SCHEDULED_OP_INTERVAL_TURNS`, `MAX_SCHEDULED_PASS_MODEL_CALLS`). Same split as `channel`: shape and section-local invariants here (closed trigger vocabulary, exactly-one op kind, cost floor, model-call ceiling, one op per trigger); the two cross-section rules — a pass's tools must be a subset of the manifest's own `[[tools]]`, and only a first-party/system manifest may declare a pass op — are `ironclaw_extension_registry::v3::validate_memory_scheduled_ops`, because only the manifest layer sees the whole file. Asset resolution for `pass.prompt` stays host-side and fail-closed, like `guidance_doc`. |
| `preference_target` | `PreferenceTargetCodec` + `PreferenceTargetEncodeRequest` — the one vendor-implemented port here. |
| `product_adapter_section` | The `[product_adapter.*]` manifest surface: `PRODUCT_ADAPTER_HOST_API_ID`/`PRODUCT_ADAPTER_SECTION_PREFIX`, `ProductAdapterSectionDeclaration` (the `Deserialize` wire shape), `ProductAdapterSection` (resolved + validated), `HostIngressRoute`, `ProductAdapterSectionError`. Arrived with WS5 from `ironclaw_assistant::adapter_registry`. Same split as `channel`: the schema and its cross-field invariants are here; the *manifest parsing* — the `HostApiManifestContract`, the raw-TOML inline-secret guard, and pairing a resolved section with its `ManifestSectionPath` — is `ironclaw_extension_registry::host_api::product_adapter` (§6.8.1), because this crate parses no manifests. |
| `recipe` | The auth recipe schema: `VendorAuthRecipe`, `OAuth2CodeRecipe`, `PkceMode`, ingress-verification recipes, and friends. |
| `runtime` | The extension runtime descriptor vocabulary: `ExtensionRuntime` and the validated `ExtensionAssetPath`/`ExtensionAssetPathError` pair that keeps a manifest-declared asset path inside its package root. Arrived 2026-08-03 with WS3 (#7065) — the carve-out that let both execution lanes drop their registry edge. Resolving a path *under* a package root needs `ironclaw_filesystem::VirtualPath`, which this crate may not name, so resolution stays the free function `ironclaw_extension_registry::resolve_asset_under` (an orphan-rule cost, recorded). |
| `state` | The installation state machine: `InstallationState`, `LifecyclePublicState`. |
| `surface` | `CapabilitySurfaceKind` — the manifest surface kinds an extension may declare. |
| `tool_adapter` | `ToolAdapter` + `RestrictedEgress` and their call/result/error vocabulary — arrived with WS1.4. |
| `verified_inbound` | The **sealed** channel/webhook mint family: `mark_request_signature_verified` and `mark_shared_secret_header_verified` — arrived with WS1.5; their `_for_tenant` variants were deleted as zero-caller dead mint surface by WS8 (2026-08-05). Every entry point consumes a `VerifiedInboundGrant`, so only the generic ingress verifier can call them. |
| `test_support` | Feature-gated: the exported channel-capability conformance suite (§11.2.10) and the in-memory egress/delivery fakes. |

## What must never be here

The registry or installation stores (`ironclaw_extension_registry`); lifecycle
execution, binding orchestration, or ingress routing
(`ironclaw_extension_host`); product workflow; WASM/MCP mechanics; vendor
names (the specificity scanner polices this crate like any other); any
implementation of a port declared here (§6.1.4's rule applies family-wide —
the `PreferenceTargetCodec` implementations live in the Slack and Telegram
packages, which is the point).

## Why none of these traits is sealed

`ironclaw_agent_loop::planner` is the workspace's sealed-strategy template: a
private `sealed::Sealed` supertrait with a closed impl list, so no crate
outside the owner can add a variant of a *strategy* the host must reason about
exhaustively. **The opposite is true here.** Every trait in this crate exists
to be implemented outside it — `PreferenceTargetCodec` by the channel
packages, `Extension` and the `ChannelIdentity*` hooks by extension
implementations and their hosts. Sealing them would forbid exactly the
extensibility the unified extension model is built on, so it is not an
omission: a trait added here is open by default, and one that genuinely needs
a closed impl set is a sign it belongs in an owner crate instead.

**The one closed thing here is not a trait — it is a constructor.**
`verified_inbound`'s mint family is sealed by a *witness*, not by a sealed
supertrait: each function consumes a
`ironclaw_host_api::product_adapter::auth::VerifiedInboundGrant`, whose only
source is `ChannelIngressVerifier::verified_inbound_grant`, which only
`ironclaw_extension_host` may implement (pinned by
`reborn_sealed_evidence_mint_ratchet`). That is deliberately the *opposite*
shape from the traits above: this crate declares what an extension may
implement, and separately declares the one thing an extension may never do —
attest that a request was verified. See PROPOSAL §6.1.2/§12.1a.

## Dependencies

`ironclaw_host_api` and nothing else internal. No framework, driver, or
runtime client — no `axum`, `reqwest`, `wasmtime`, `libsql`, `tokio`.
Validation failures are reported as
`ironclaw_host_api::error::HostApiError`; this crate deliberately does **not**
introduce a parallel error type for the same contract failures.

## Admission tests

Three architecture tests hold the line, all runnable with
`cargo test -p ironclaw_architecture_tests`:

- `reborn_dependency_boundaries.rs` — the §11.2.3 internal-dependency
  allowlist (`ironclaw_host_api` only, an allowlist so a future edge cannot
  slip past a list of today's offenders), the external framework/driver deny
  shared with the other contracts crates, and the crate's `BoundaryRule`.
- `reborn_extension_contract_location_scan.rs` — the §11.2.4 port-location
  rule: one definition per contract workspace-wide, and one import path (no
  crate re-exports one). Read its module doc before adding a `pub use`
  anywhere that names a type from here; the dual-path re-export is exactly the
  defect it exists to prevent, and the extension tier had three live instances
  of it.
- `reborn_extension_specificity.rs` — vendor-name scanning, which reaches this
  crate automatically through `cargo metadata`.

## Why `hosted_mcp` lives here

It arrived on `main` with #6930 as `ironclaw_host_api::hosted_mcp` and had to
move: `hosted_mcp` names `LifecyclePackageId` and `package_lifecycle` names
`hosted_mcp::RegisterHostedMcpRequest`. Mutually referencing modules are fine
inside one crate — but once `package_lifecycle` left `ironclaw_host_api`,
keeping `hosted_mcp` there would have required
`host_api -> extension_contracts`, and `host_api` may hold no internal
dependency at all. `hosted_mcp` lives here, which is also where the charter
puts it: it is registration input describing what an installable extension
*is*, and the module's own doc already says the extension host owns every
behavior around it.

WS1.4 then split the coupling the only way the one-way street allows.
`package_lifecycle` went to `ironclaw_product_contracts` (PROPOSAL §6.1.3,
which names its UI projections explicitly), and the one type `hosted_mcp`
structurally needs — `LifecyclePackageId` — stayed on this side in the new
`lifecycle_id` module, with `package_lifecycle` importing it from below. The
alternative, dragging `hosted_mcp` up into `product_contracts` with the
projections, would have handed `ironclaw_mcp`,
`ironclaw_extension_registry`, and `ironclaw_auth` a product-tier edge —
exactly what §6.1.2 exists to prevent.

## Resolved placements (WS1.4)

`package_lifecycle` **left** for `ironclaw_product_contracts`, which is where
PROPOSAL §6.1.3 assigns it; WS1.3 had it here only as a forced co-mover and
said so. Nothing here depended on it staying.

The channel adapter vocabulary, `ToolAdapter`, and `RestrictedEgress`
**arrived**, unblocked
exactly as WS1.3 predicted: `host_api::product_surface` moved to
`ironclaw_product_contracts`, so nothing that stays in `ironclaw_host_api`
names them any more. `egress` and `external` came with them (§6.1.2 names both
in its "fed by" list, and the adapter cone types every one of them).

`auth_prompt` arrived for a reason §6.1.3's prose did not anticipate: it lists
"auth/approval prompt-view DTOs" as product-tier, but the channel output
traits' `OutboundPart::AuthPrompt` carries `AuthPromptView`, and both shipped
message-channel packages call `render_channel_auth_prompt` from their send
methods. The approval half
stayed in `product_contracts::outbound`. The module doc records the one
deliberate consequence: two ~15-line display-text validators exist in both
crates rather than making a generic validator part of this crate's public API.

## Validation

- Fast local check: `cargo test -p ironclaw_extension_contracts`
- Boundary/scan gates: `cargo test -p ironclaw_architecture_tests`
