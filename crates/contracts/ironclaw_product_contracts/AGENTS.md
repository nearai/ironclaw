# ironclaw_product_contracts — working rules

Canonical crate guidance (the crate's `CLAUDE.md` is a pointer here).
Orientation and public surface: [`README.md`](./README.md). Family boundary
and admission test: [`../AGENTS.md`](../AGENTS.md). Carved out of
`ironclaw_host_api` (and `ironclaw_extension_contracts`) by WS1.4 of the
target architecture (PROPOSAL §6.1.3,
`docs/internal/reborn/target-architecture/families/contracts.md`).

## What belongs here

A type is admitted iff all four parts of the contracts-family test hold
(family `AGENTS.md`): it names a concept crossing the product boundary; it is
neutral across vendor, runtime, storage, and deployment; two or more consumers
need it without importing an owner; it carries no execution, persistence,
policy engine, or workflow.

Today that is **thirty-four** shipped modules (plus the dev-only
`test_support`, gated behind `#[cfg(any(test, feature = "test-support"))]`;
`src/lib.rs` is the source of truth for the list). ✎ *Re-measured 2026-08-10
by the unified-channel-model train: `web_app` was replaced by
`notification_setup`, and `session_ingress` joined — counted on `src/lib.rs`,
thirty-four `pub mod` lines minus the gated `test_support`.* ✎ *Corrected 2026-08-05:
this read "twenty-six", which was already wrong before `project_service` was
added — counted on `src/lib.rs`, thirty modules shipped and the table below
documented twenty-six. The four the table has never carried are
`actor_identity`, `approval_prompt`, `binding` and `channel_workflow`;
recorded here rather than back-filled, because inventing charter sentences for
four modules is not this slice's to do. The count is now measured, not
asserted.*

| Module | Owns |
| --- | --- |
| `surface` | The membrane: `ProductSurface`, `BoundProductSurface`, `ProductSurfaceCaller`, the invoke/query/stream DTOs, `ChannelInboundProductSurface` + its admission outcome types, and the `ProductSurfaceError` family every transport renders. |
| `inbound` | The inbound envelope/payload/ack/rejection DTOs a product surface admits, and the channel-inbound classification vocabulary. |
| `outbound` | The product projection wire: `ProductOutboundEnvelope`, `ProductProjectionState`/`Item`, the approval prompt views, capability activity views, progress views, `ProjectionCursor`. |
| `projection` | The projection read/subscribe ports and their request DTOs (`ProjectionStream`, `ProjectionStreamSubscription`). |
| `interaction_commands` | The channel-neutral interaction-reply grammar (`parse_interaction_resolution_text`). |
| `operator_llm` | The operator LLM-administration port (`LlmConfigService`), the active-model read port (`ActiveModelReader`), the provider-menu and login/probe wire vocabulary, `LlmConfigServiceError`, and its projection onto `ProductSurfaceError`. Implemented by `ironclaw_operator`; the `llm_config` view descriptor and the "no service wired" error stay with product. |
| `package_lifecycle` | Package/extension lifecycle projection vocabulary (`Lifecycle*`, `ChannelConnectStrategy`, `ChannelConfigField`) — see the ruling below. |
| `ironhub` | The IronHub link port (`IronhubLinkService`), its register/install-delivery request and result bodies, `IronhubLinkError`, and the `ironhub.deliver_install` command descriptor. |
| `lifecycle_service` | The lifecycle product service port (`LifecycleProductService`) and its caller contexts. Implemented by `ironclaw_extension_manager` (WS2.4); the *authority* it calls — the only writer of lifecycle state — stayed in `ironclaw_extension_host`. |
| `delivery` | The delivery-resolution ports: `ChannelDeliveryResolver`, `ResolvedChannelDelivery`, `DeliveryReplyContextSource`. The coordinator itself is product's. |
| `account_setup` | `AccountConnectionStatusSource` + the extension account-setup descriptor/notice/error vocabulary. The declaration registry is product's (it holds mutable state). |
| `channel_config` | `ChannelConfigProductService` — per-extension `[channel.config]` operator config, implemented over the installation store. |
| `prompt_source` | Gate-prompt enrichment ports: `ApprovalPromptContextSource`, `BlockedAuthPromptSource`, `BlockedAuthPromptRequest`. Rendering stays in product. |
| `command` | `ProductCommandContext` (the authority-bearing dispatch context) and the `CommandActorRoleResolver` admission port. |
| `action` | Inbound-action identity (`ProductActionId`), the bounded product tokens, and `ActionFingerprintKey`. The ledger record and saga are product's. |
| `admin_users` | The `AdminUserService` port, its records, its error taxonomy, and the `Reborn*` HTTP wire DTOs that wrap them (moved here by the WS5 inversion — §6.1.3's frozen inventory is the concrete *constants*, not the request/response bodies). |
| `operator_tools` | `RebornOperatorToolCatalog` + `RebornOperatorToolInfo`. |
| `views` | The generic product-view conduit's `RebornViewDescriptor`/`Query`/`Page` and the `RebornViewProvider` port. The typed `ProductView` wrapper sits in `descriptors` with the other two operation shapes. |
| `descriptors` | The three `ProductSurface` operation shapes — `ProductSurfaceCommandDescriptor`, `ProductCapabilityDescriptor`, `ProductView`, `EmptyProductCommandInput` — plus their encode/decode glue. The *types*; product keeps the concrete constants as its frozen inventory. |
| `inbound_requests` | The browser/API request bodies a transport hands to `ProductSurface` (`ProductSubmitTurnRequest`, `ProductCreateThreadRequest`, the cancel/gate/retry/setup/list bodies, `ProductInboundAttachment`). Field shapes and the `serde` contract only — normalization stays in product. |
| `product_wire` | The `Reborn*` product wire DTO family every product transport serializes across the boundary. Payload vocabulary only: no service, handler, or projection reducer. |
| `workspace_views` | Project and filesystem-browse wire vocabulary for the Projects page and the Workspace/Files explorer. The two *filesystem* read ports stayed in product (`ProjectFilesystemReader` cannot move — it takes an `ironclaw_threads::ThreadScope`; `FilesystemBrowseReader` has not, because its implementor is composition and hoisting removes no edge). |
| `project_service` | The project management + membership-ACL port: `ProjectService`, `ProjectServiceError`. **Implemented *below* product** — `ironclaw_identity::projects::service::RebornProjectService`, which is why the declaration had to leave `ironclaw_assistant` (a `products` declaration is unnameable from a `substrates` implementor). Arrived 2026-08-05, PROPOSAL §12.13 D-P; the identity crate's one-entry allowlist widening is D-Q. Its DTOs live next door in `workspace_views`; nothing here is a repository, a role resolver, or an access decision. |
| `operator_secrets` | The operator control plane's secret-**value** port (`OperatorSecretValueStore`) and its opaque error. Implemented by `ironclaw_composition` — assembly is the only layer that may name both this port and `ironclaw_secrets` (PROPOSAL §8.2's product row). **Deliberately not a re-export of `SecretStorePort`:** no `ResourceScope` argument (the implementor fixes the operator scope), no lease/consume protocol, and an error carrying only a `&'static str` classification. Widening it back toward the substrate's shape undoes what CHECKLIST WS3 bought. |
| `operator_service` | The deployment-operator control plane's three ports — `OperatorStatusService`, `OperatorLogsService`, `OperatorServiceLifecycleService` — their wire DTOs, and the log-context bound (`normalize_operator_log_context_value`). Implemented by `ironclaw_operator` except readiness status, which is composition's. Product keeps the `Unsupported*`/`Static*` doubles, the frozen view descriptors, and the operator *command-plane* envelope that wraps these DTOs. |
| `error` | `ProductOperationFailure` — the error a product-side port fails with, and its projection onto `ProductSurfaceError`. Product's `ProductSurfaceFailure` is the superset and absorbs it; see the ruling below. |
| `notification_setup` | The generic per-channel notification-setup operation descriptors (§7b of the unified channel model): `NOTIFICATION_SETUP_STATUS_VIEW` and the `notification_setup.enable` / `notification_setup.disable` command descriptors (+ ids), all parameterized by `extension_id` with channel-opaque payload/detail documents. Descriptors only — dispatch to the channel's adapter stays in `ironclaw_assistant`; enrollment behavior and storage stay behind the adapter. |
| `session_ingress` | `SessionChannelDirectory` — the port telling the session-inbound lane whether an extension is the deployment's authenticated-session channel. Implemented by `ironclaw_extension_host` over the deployment channel registry. |
| `shared_admission` | Shared-conversation admission: `SharedConversationAdmission` + `ProductConversationRouteKey` and its request. Fail-closed connected-channel gating, implemented by `ironclaw_extension_host` over `[channel.config]`. |

## What must never be here

The `ProductSurface` *implementation* and the frozen inventory of concrete
commands/views/capabilities (`ironclaw_assistant`); any handler, admission,
delivery, or projection-reducer logic; HTTP of any kind (`axum` lives in
`ironclaw_host_ingress`); vendor names outside the censused `operator_llm`
bound (below); any implementation of a port declared here (§6.1.4's rule
applies family-wide).

## Dependencies

`ironclaw_host_api` and `ironclaw_extension_contracts` — the latter is the
one-way street §6.1.3 grants explicitly, "for channel-facing DTO reuse", and
it is why `surface` can name `ChannelAdapter`/`RestrictedEgress` and
`outbound` can name the auth-prompt views. Nothing else internal, and no
framework, driver, or runtime client.

`tokio` appears with the `sync` feature only, for the two continuation handles
a transport holds open across a client connection
(`ProductSurfaceEventSubscription`, `ProjectionStreamSubscription`). WS1's
"evict behavior from `host_api` to product" row owns the `tokio::sync::mpsc`
projection type by name; this crate inherited the dependency with the types
rather than adding it. `secrecy` is the other documented external: the
secret-bearing wire fields must not degrade to `Debug`-printable `String`s
(see the manifest comment).

## Admission tests

Architecture tests hold the line, all runnable with
`cargo test -p ironclaw_architecture_tests`:

- `reborn_dependency_boundaries.rs` — the §11.2.3 internal-dependency
  allowlist (`ironclaw_host_api` + `ironclaw_extension_contracts`, an
  allowlist so a future edge cannot slip past a list of today's offenders),
  the external framework/driver deny shared with the other contracts crates,
  and the crate's `BoundaryRule`.
- `reborn_product_contract_location_scan.rs` — the §11.2.4 port-location rule:
  one definition per contract workspace-wide, and one import path for the
  *ports*. Read its module doc before adding a `pub use` anywhere that names a
  trait from here; it also records exactly which re-exports are deliberately
  out of scope and why.
- `reborn_service_method_freeze_ratchet.rs` — the `ProductSurface` method set
  (`invoke`, `query`, `stream_events`) stays frozen, and `ironclaw_assistant`
  does not grow a second local product-surface trait.
- `reborn_contracts_vendor_census.rs` — the `operator_llm` vendor bound as an
  exact roster (below).

## Rulings and known placements

**`package_lifecycle` came here, per §6.1.3.** WS1.3 moved it into
`ironclaw_extension_contracts` as a forced co-mover and recorded the placement
as interim rather than a decision. The § text is unambiguous — §6.1.3's Owns
list names "`package_lifecycle` UI projections" and §6.1.2's does not — and
the code agrees: `LifecycleProductAction`/`LifecycleProductResponse` are the
product command and projection vocabulary consumed by
`LifecycleProductService`, which §6.1.3 also assigns here. The move costs
nothing because this crate may depend on `ironclaw_extension_contracts`, so
the four §6.1.2 types it is typed on (`InstallationState`,
`LifecyclePublicState`, `ChannelPresentation`, `CapabilitySurfaceKind`) stay
reachable from below.

**Three types that read product-tier but stay in `ironclaw_host_api`.**
`ProductAdapterError` (+ the `RedactedString` family), the adapter identity
newtypes (`ProductAdapterId`, `AdapterInstallationId`, `ProductSurfaceKind`),
and `ProtocolAuthEvidence`. `host_api` may hold no internal dependency, and
each is named by something that stays there — `host_api::user_identity` names
`AdapterInstallationId`, `host_api::product_adapter::auth` names
`ProductAdapterError`. Both contracts tiers reach them downward, which is the
only placement that serves both. `ProtocolAuthEvidence` **stayed** when WS1's
sealed-evidence-minting row landed: that row split the *mint family* by trust
role (channel/webhook to `ironclaw_extension_contracts::verified_inbound`,
bearer/session kept in `ironclaw_host_api`) and replaced the `host-auth-mint`
feature with witness grants, but §6.1.1 owns the evidence type itself and it
did not move. Product is not a minter, and WS1.5 deleted both of
`ironclaw_assistant`'s re-export paths to the family.

**The auth-prompt view family went to the extension tier, not here.** §6.1.3
lists "auth/approval prompt-view DTOs" together, but at this base only the
*auth* half is named by an adapter signature: `ChannelAdapter`'s own
`OutboundPart::AuthPrompt` carries an `AuthPromptView`, and both shipped
channel packages call `render_channel_auth_prompt` from `deliver`. It lives in
`ironclaw_extension_contracts::auth_prompt`; the approval half
(`ApprovalPrompt*View`), which only product and WebUI reach, stayed in
`outbound` here.

**The twelve ports WS2 relocated, and the five it could not.** The
`extension_host` port-inversion row moved every product-declared port the
extension host reaches whose signature this crate may legally name. **Ten of
them the extension side implements** — those are the ones
`reborn_extension_host_port_inversion.rs::INVERTED_PORT_IMPLEMENTORS`
enumerates and pins, each *with its implementing crate*, because WS2.4 split
the extension-management product face out of the host and four of the ten went
with it:

| Port | Implemented by |
|---|---|
| `AccountConnectionStatusSource` | `ironclaw_extension_host` |
| `ApprovalPromptContextSource` | `ironclaw_extension_host` |
| `BlockedAuthPromptSource` | `ironclaw_extension_host` |
| `ChannelDeliveryResolver` | `ironclaw_extension_host` |
| `CommandActorRoleResolver` | `ironclaw_extension_host` |
| `DeliveryReplyContextSource` | `ironclaw_extension_host` |
| `SharedConversationAdmission` (inverted WS2.2 as `ProductConversationSubjectRouteResolver`; reshaped admission-only when shared-route subjects retired) | `ironclaw_extension_host` |
| `ChannelConfigProductService` | `ironclaw_extension_manager` |
| `LifecycleProductService` | `ironclaw_extension_manager` |
| `RebornViewProvider` | `ironclaw_extension_manager` |

**Two more it only consumes**, implemented in `ironclaw_composition`, and they
moved for the same reason — a port whose implementation sits outside product
does not belong inside it: `AdminUserService`, `RebornOperatorToolCatalog`.
Quote that test rather than this list when the count matters; the list here is
prose and the test is the enforced inventory. Five stayed, and each for the
same mechanical reason rather than a judgement call — **this crate's
dependency allowlist is `ironclaw_host_api` + `ironclaw_extension_contracts`
and nothing else internal**, so a port whose signature names a type from
`ironclaw_auth`, `ironclaw_threads`, `ironclaw_turns`, or
`ironclaw_conversations` cannot be declared here until that type is narrowed
out of it: `AuthChallengeProvider` and `ChannelConnectionService` and
`ExtensionCredentialSetupService` (auth credential vocabulary),
`ConversationBindingService` (its `ResolveBindingRequest`/`ResolvedBinding`
sit in product beside the route-kind grammar that derives them), and
`ProductActorUserResolver` (`ResolvedProductActorUser` carries
`ironclaw_conversations::ExternalActorBindingEpoch`). *WS2.2 corrected the
last two reasons: they named `ProductSurfaceFailure`, which no longer blocks
anything.* The residue is enumerated with its reasons and held shrink-only by
`crates/app/ironclaw_architecture_tests/tests/reborn_extension_host_port_inversion.rs`;
**do not add a row there** — narrow the signature or move the type instead.

The sibling gate `reborn_operator_port_inversion.rs` does the same job for
`ironclaw_operator`, and its residue is **empty**: every port that crate
implements is declared here — the full operator service set
(`LlmConfigService`, `ActiveModelReader`, `OperatorLogsService`,
`OperatorServiceLifecycleService`, `OperatorStatusService`) landed with the
WS5 operator row (#7004 via #7018), and the gate additionally proves, through
`cargo metadata`, that `ironclaw_operator` names no `ironclaw_assistant`
dependency under any kind. Adding a product-declared port for the operator to
implement will fail there before it fails anywhere else.

**The error a port fails with lives here too**
(`error::ProductOperationFailure`, WS2.2). It is the boundary vocabulary — six
variants whose payloads are a plain `String` or nothing — so a crate below
product can describe its own failure without naming product's workflow error.
`ironclaw_assistant::ProductSurfaceFailure` is the strict superset carrying
the turn-kernel and interaction payloads only the workflow crate produces, and
it absorbs this type with a total `From`. Two rules follow, both pinned by
tests:

- **Do not add a kernel-typed variant here.** `ProductSurfaceFailure` could
  not move precisely because two of its variants carry
  `ironclaw_turns::TurnError`; reintroducing one re-creates the blocker, and
  the port-inversion scan fails on any mention of
  `TurnError`/`ironclaw_turns`/`ironclaw_auth`/`ironclaw_threads`/
  `ironclaw_conversations`/`ironclaw_assistant` in `src/error.rs`.
- **The projection to `ProductSurfaceError` is defined here once.** Product's
  `lifecycle_product_surface_error` delegates its matching arms to it rather
  than repeating the status choices, so the WebUI cannot get one answer
  through product's lifecycle service and a different one through the
  extension host's. Only the logging stays with each caller — this crate may
  not log.

## Vendor neutrality has one bounded exception, and it is not a licence

`operator_llm` names NEAR AI and OpenAI Codex — three method names
(`start_nearai_login`, `complete_nearai_wallet_login`, `start_codex_login`)
and six DTOs (`NearAi*` ×5, `CodexLoginStart`). **Resolved 2026-08-02
(PROPOSAL §12.11 D-E):** §8.2's vendor rule now sanctions LLM-vendor
administration vocabulary in `ironclaw_product_contracts::operator_llm` —
that module and nowhere else in this family. The reason the port could not be
narrowed is protocol, not wire compatibility: NEAR AI SSO, NEAR wallet
NEP-413, and Codex device-code are three disjoint protocols, so a neutral port
collapses to an untyped `serde_json::Value` payload, which
`.claude/rules/types.md` forbids. The bound is enforced, not review
discipline — `reborn_contracts_vendor_census.rs` (#7150) freezes the exact
roster (6 DTOs / 3 methods / 2 vendors):

- **Do not add a seventh vendor name here.** A fourth provider login arrives
  as a package or behind a shape that adds no vendor-named method or DTO.
- The two names the specificity scanner can see —
  `NearAiAuthProvider::{Github, Google}` — carry allowlist entries that were
  **repointed** from `ironclaw_assistant`, not added. The baseline did not
  move and must not move for this module.

## Deferred by design (not missing)

**The line that did *not* move, and must not be crossed casually.** §6.1.3
gives this crate the descriptor *types* and explicitly withholds product's
concrete constants, which stay in product as the frozen inventory (live count
26 commands / 35 capabilities / 37 views — §12.11 D-B). Those constants are
what a route handler actually names to call the surface, so they are the whole
reason `ironclaw_webui` (91 of them) and `ironclaw_openai_compat` (3) still
depend on `ironclaw_assistant` after the inversion.
`reborn_transport_product_boundary.rs` pins the split in **both** directions —
the moved vocabulary must be here, and a sample of the inventory must **not**
be — so "finish the row" cannot quietly mean "move the inventory too". That is
an owner decision recorded on the CHECKLIST WS5 `webui` row, not a cleanup.

`ProductCommandAdmissionService` stays in `ironclaw_assistant`, and since
§12.11 D-D that is a ruling rather than an accident: the port has three
implementors, one call site, and no consumer outside that crate, so it fails
the family's own two-consumer admission test (a port whose caller and
implementor are the same crate has one consumer). Its earlier §6.1.3
assignment is withdrawn; do not move it here without a second, external
consumer.

## Validation

- Fast local check: `cargo test -p ironclaw_product_contracts`
- Boundary/scan/freeze/census gates:
  `cargo test -p ironclaw_architecture_tests`
