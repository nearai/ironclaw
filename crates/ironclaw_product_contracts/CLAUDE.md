# ironclaw_product_contracts

The product tier's neutral contract: the `ProductSurface` membrane every
transport calls through, the wire DTOs that cross it, and the product-side
ports whose implementations sit beside or below product. Carved out of
`ironclaw_host_api` (and `ironclaw_extension_contracts`) by WS1.4 of the target
architecture (PROPOSAL §6.1.3,
`docs/reborn/target-architecture/families/contracts.md`).

## What belongs here

A type is admitted iff all four hold (the contracts-family test, §6.1):

1. it names a concept crossing the product boundary;
2. it is neutral across vendor, runtime, storage, and deployment;
3. two or more consumers need it without importing an owner;
4. it carries no execution, persistence, policy engine, or workflow.

Today that is seven modules:

| Module | Owns |
| --- | --- |
| `surface` | The membrane: `ProductSurface`, `BoundProductSurface`, `ProductSurfaceCaller`, the invoke/query/stream DTOs, `ChannelInboundProductSurface` + its admission outcome types, and the `ProductSurfaceError` family every transport renders. |
| `inbound` | The inbound envelope/payload/ack/rejection DTOs a product surface admits, and the channel-inbound classification vocabulary. |
| `outbound` | The product projection wire: `ProductOutboundEnvelope`, `ProductProjectionState`/`Item`, the approval prompt views, capability activity views, progress views, `ProjectionCursor`. |
| `projection` | The projection read/subscribe ports and their request DTOs (`ProjectionStream`, `ProjectionStreamSubscription`). |
| `interaction_commands` | The channel-neutral interaction-reply grammar (`parse_interaction_resolution_text`). |
| `operator_llm` | The operator LLM menu vocabulary. |
| `package_lifecycle` | Package/extension lifecycle projection vocabulary (`Lifecycle*`, `ChannelConnectStrategy`, `ChannelConfigField`) — see the ruling below. |

## What must never be here

The `ProductSurface` *implementation* and the frozen inventory of concrete
commands/views/capabilities (`ironclaw_product`); any handler, admission,
delivery, or projection-reducer logic; HTTP of any kind (`axum` lives in
`ironclaw_host_ingress`); vendor names (the specificity scanner polices this
crate like any other); any implementation of a port declared here (§6.1.4's
rule applies family-wide).

## Dependencies

`ironclaw_host_api` and `ironclaw_extension_contracts` — the latter is the
one-way street §6.1.3 grants explicitly, "for channel-facing DTO reuse", and it
is why `surface` can name `ChannelAdapter`/`RestrictedEgress` and `outbound` can
name the auth-prompt views. Nothing else internal, and no framework, driver, or
runtime client.

`tokio` appears with the `sync` feature only, for the two continuation handles a
transport holds open across a client connection (`ProductSurfaceEventSubscription`,
`ProjectionStreamSubscription`). WS1's "evict behavior from `host_api` to
product" row owns the `tokio::sync::mpsc` projection type by name; this crate
inherited the dependency with the types rather than adding it.

## Admission tests

Three architecture tests hold the line, all runnable with
`cargo test -p ironclaw_architecture`:

- `reborn_dependency_boundaries.rs` — the §11.2.3 internal-dependency allowlist
  (`ironclaw_host_api` + `ironclaw_extension_contracts`, an allowlist so a
  future edge cannot slip past a list of today's offenders), the external
  framework/driver deny shared with the other contracts crates, and the crate's
  `BoundaryRule`.
- `reborn_product_contract_location_scan.rs` — the §11.2.4 port-location rule:
  one definition per contract workspace-wide, and one import path for the
  *ports*. Read its module doc before adding a `pub use` anywhere that names a
  trait from here; it also records exactly which re-exports are deliberately
  out of scope and why.
- `reborn_service_method_freeze_ratchet.rs` — the `ProductSurface` method set
  (`invoke`, `query`, `stream_events`) stays frozen, and `ironclaw_product` does
  not grow a second local product-surface trait.

## Rulings and known placements

**`package_lifecycle` came here, per §6.1.3.** WS1.3 moved it into
`ironclaw_extension_contracts` as a forced co-mover and recorded the placement
as interim rather than a decision. The § text is unambiguous — §6.1.3's Owns
list names "`package_lifecycle` UI projections" and §6.1.2's does not — and the
code agrees: `LifecycleProductAction`/`LifecycleProductResponse` are the product
command and projection vocabulary consumed by `LifecycleProductService`, which
§6.1.3 also assigns here. The move costs nothing because this crate may depend
on `ironclaw_extension_contracts`, so the four §6.1.2 types it is typed on
(`InstallationState`, `LifecyclePublicState`, `ChannelPresentation`,
`CapabilitySurfaceKind`) stay reachable from below.

**Three types that read product-tier but stay in `ironclaw_host_api`.**
`ProductAdapterError` (+ the `RedactedString` family), the adapter identity
newtypes (`ProductAdapterId`, `AdapterInstallationId`, `ProductSurfaceKind`),
and `ProtocolAuthEvidence`. `host_api` may hold no internal dependency, and each
is named by something that stays there — `host_api::user_identity` names
`AdapterInstallationId`, `host_api::product_adapter::auth` names
`ProductAdapterError`. Both contracts tiers reach them downward, which is the
only placement that serves both. `ProtocolAuthEvidence` additionally waits on
WS1's sealed-evidence-minting row, which owns its move and the `host-auth-mint`
feature deletion.

**The auth-prompt view family went to the extension tier, not here.** §6.1.3
lists "auth/approval prompt-view DTOs" together, but at this base only the
*auth* half is named by an adapter signature: `ChannelAdapter`'s own
`OutboundPart::AuthPrompt` carries an `AuthPromptView`, and both shipped channel
packages call `render_channel_auth_prompt` from `deliver`. It lives in
`ironclaw_extension_contracts::auth_prompt`; the approval half
(`ApprovalPrompt*View`), which only product and WebUI reach, stayed in
`outbound` here.

## Deferred by design (not missing)

§6.1.3's Owns list also names the product-side **ports** whose implementations
live beside product — delivery resolution, command admission, the operator
service set, `LifecycleProductService`, `AccountConnectionStatusSource`,
`ChannelConfigProductService` — and the command/view/capability descriptor
types. Those are sourced from `ironclaw_product`, not from `ironclaw_host_api`,
and CHECKLIST WS2 ("flip `extension_host`'s implemented ports to
`product_contracts`/`extension_contracts` definitions") and WS6 ("`operator`
implements `product_contracts` ports") own them by name. They land in this
crate; they land with the PRs that repoint their implementors, because moving a
port definition without its implementation buys no dependency-edge removal. The
WS1.4 PR body carries the per-port movability analysis those rows need.
