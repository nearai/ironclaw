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

Today that is seventeen shipped modules (plus the dev-only `test_support`, gated behind `#[cfg(any(test, feature = "test-support"))]`; `src/lib.rs` is the source of truth for the list):

| Module | Owns |
| --- | --- |
| `surface` | The membrane: `ProductSurface`, `BoundProductSurface`, `ProductSurfaceCaller`, the invoke/query/stream DTOs, `ChannelInboundProductSurface` + its admission outcome types, and the `ProductSurfaceError` family every transport renders. |
| `inbound` | The inbound envelope/payload/ack/rejection DTOs a product surface admits, and the channel-inbound classification vocabulary. |
| `outbound` | The product projection wire: `ProductOutboundEnvelope`, `ProductProjectionState`/`Item`, the approval prompt views, capability activity views, progress views, `ProjectionCursor`. |
| `projection` | The projection read/subscribe ports and their request DTOs (`ProjectionStream`, `ProjectionStreamSubscription`). |
| `interaction_commands` | The channel-neutral interaction-reply grammar (`parse_interaction_resolution_text`). |
| `operator_llm` | The operator LLM menu vocabulary. |
| `package_lifecycle` | Package/extension lifecycle projection vocabulary (`Lifecycle*`, `ChannelConnectStrategy`, `ChannelConfigField`) — see the ruling below. |
| `lifecycle_service` | The lifecycle product service port (`LifecycleProductService`) and its caller contexts. Implemented by `ironclaw_extension_host` — the only crate that may write lifecycle state. |
| `delivery` | The delivery-resolution ports: `ChannelDeliveryResolver`, `ResolvedChannelDelivery`, `DeliveryReplyContextSource`. The coordinator itself is product's. |
| `account_setup` | `AccountConnectionStatusSource` + the extension account-setup descriptor/notice/error vocabulary. The declaration registry is product's (it holds mutable state). |
| `channel_config` | `ChannelConfigProductService` — per-extension `[channel.config]` operator config, implemented over the installation store. |
| `prompt_source` | Gate-prompt enrichment ports: `ApprovalPromptContextSource`, `BlockedAuthPromptSource`, `BlockedAuthPromptRequest`. Rendering stays in product. |
| `command` | `ProductCommandContext` (the authority-bearing dispatch context) and the `CommandActorRoleResolver` admission port. |
| `action` | Inbound-action identity (`ProductActionId`), the bounded product tokens, and `ActionFingerprintKey`. The ledger record and saga are product's. |
| `admin_users` | The `AdminUserService` port, its records, and its error taxonomy. The `Reborn*` HTTP wire DTOs stay with product's frozen surface. |
| `operator_tools` | `RebornOperatorToolCatalog` + `RebornOperatorToolInfo`. |
| `views` | The generic product-view conduit's `RebornViewDescriptor`/`Query`/`Page` and the `RebornViewProvider` port. `ProductView` (the typed declaration wrapper) stays with product's frozen inventory. |

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
only placement that serves both. `ProtocolAuthEvidence` **stayed** when WS1's
sealed-evidence-minting row landed: that row split the *mint family* by trust
role (channel/webhook to `ironclaw_extension_contracts::verified_inbound`,
bearer/session kept in `ironclaw_host_api`) and replaced the `host-auth-mint`
feature with witness grants, but §6.1.1 owns the evidence type itself and it did
not move. Product is not a minter, and WS1.5 deleted both of `ironclaw_product`'s
re-export paths to the family.

**The auth-prompt view family went to the extension tier, not here.** §6.1.3
lists "auth/approval prompt-view DTOs" together, but at this base only the
*auth* half is named by an adapter signature: `ChannelAdapter`'s own
`OutboundPart::AuthPrompt` carries an `AuthPromptView`, and both shipped channel
packages call `render_channel_auth_prompt` from `deliver`. It lives in
`ironclaw_extension_contracts::auth_prompt`; the approval half
(`ApprovalPrompt*View`), which only product and WebUI reach, stayed in
`outbound` here.

**The eleven ports WS2's first row relocated, and the six it could not.** The
`extension_host` port-inversion row moved every product-declared port the
extension host reaches whose signature this crate may legally name. **Nine of
them `extension_host` itself implements** — those are the ones
`reborn_extension_host_port_inversion.rs::INVERTED_PORTS` enumerates and pins:
`AccountConnectionStatusSource`, `ApprovalPromptContextSource`,
`BlockedAuthPromptSource`, `ChannelConfigProductService`,
`ChannelDeliveryResolver`, `CommandActorRoleResolver`,
`DeliveryReplyContextSource`, `LifecycleProductService`, `RebornViewProvider`.
**Two more it only consumes**, implemented in `ironclaw_reborn_composition`, and
they moved for the same reason — a port whose implementation sits outside
product does not belong inside it: `AdminUserService`,
`RebornOperatorToolCatalog`. Quote that test rather than this list when the
count matters; the list here is prose and the test is the enforced inventory.
Six stayed, and each for
the same mechanical reason rather than a judgement call — **this crate's
dependency allowlist is `ironclaw_host_api` + `ironclaw_extension_contracts`
and nothing else internal**, so a port whose signature names a type from
`ironclaw_auth`, `ironclaw_threads`, `ironclaw_turns`, or
`ironclaw_conversations` cannot be declared here until that type is narrowed
out of it: `AuthChallengeProvider` and `ChannelConnectionService` and
`ExtensionCredentialSetupService` (auth credential vocabulary),
`ConversationBindingService` and `ProductActorUserResolver` and
`ProductConversationSubjectRouteResolver` (they error with
`ironclaw_product::ProductSurfaceFailure`, which carries `ironclaw_turns::TurnError`
on two variants). The residue is enumerated with its reasons and held
shrink-only by
`crates/ironclaw_architecture/tests/reborn_extension_host_port_inversion.rs`;
**do not add a row there** — narrow the signature or move the type instead.

## Deferred by design (not missing)

✎ **2026-08-01, the WS5 transport inversion discharged most of this.** The
command/view/capability **descriptor types** now live in `descriptors`; the
inbound request bodies in `inbound_requests`; the `Reborn*` response bodies in
`product_wire`; the operator LLM-admin DTOs in `operator_llm`; the admin-user
wire DTOs in `admin_users`; the project/filesystem-browse DTOs in
`workspace_views`. What is still sourced from `ironclaw_product` and owed to the
WS5 **`operator`** row is the operator service *ports* themselves
(`LlmConfigService`, `ActiveModelReader`, `OperatorLogsService`,
`OperatorServiceLifecycleService`, `OperatorStatusService`) — they land with the
PR that repoints their implementor, because moving a port definition without its
implementation buys no dependency-edge removal.

**The line that did *not* move, and must not be crossed casually.** §6.1.3 gives
this crate the descriptor *types* and explicitly withholds "product's 27/33/18
concrete constants, which stay in product as the frozen inventory". Those
constants are what a route handler actually names to call the surface, so they
are the whole reason `ironclaw_webui` (91 of them) and
`ironclaw_reborn_openai_compat` (3) still depend on `ironclaw_product` after the
inversion. `reborn_transport_product_boundary.rs` pins the split in **both**
directions — the moved vocabulary must be here, and a sample of the inventory
must **not** be — so "finish the row" cannot quietly mean "move the inventory
too". That is an owner decision recorded on the CHECKLIST WS5 `webui` row, not a
cleanup.

`ProductCommandAdmissionService` is a special case worth recording rather than
deciding: §6.1.3 names its "shape" for this crate, but `admit` takes a
`&ProductCommand`, and §6.9.1 keeps the command grammar with product's frozen
inventory. One of the two sections has to give; neither WS1.4 nor WS2's first
row had standing to choose, so the port stayed in `ironclaw_product`.
