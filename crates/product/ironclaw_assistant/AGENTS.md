# Agent Map — ironclaw_assistant

Working rules for the product-facing orchestration crate (issue #3280).
Orientation lives in `README.md`; family rules in `crates/product/AGENTS.md`.

**Gate-pinned:** the `reborn_services` module-charter map below is enforced by
`tests/reborn_services_module_charter.rs` — edit this file only with
`cargo test -p ironclaw_assistant` green, and do not reflow the map.

## Start here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these local contracts as the source of truth before changing behavior:
  - `tests/product_surface_contract.rs`
  - `tests/approval_interaction_contract.rs`
  - `tests/inbound_turn_contract.rs`
  - `tests/product_surface_inbound_contract.rs`
  - `tests/reborn_services_contract.rs`

## Purpose

Sits between product adapters and host-layer Reborn services. Owns the product
action orchestration so adapters (Web, API, CLI, Telegram, etc.) do not each
reimplement binding resolution, message staging, idempotency, busy/deferred
handling, gate routing, mission routing, and redacted acknowledgements. Also
owned here: project access workflow, runtime communication-context assembly,
blocked-auth resume fanout over typed host ports, and the WebUI-facing Reborn
service over thread, turn, and projection ports.

## Key types

| Type | Role |
|------|------|
| `DefaultProductSurface` | Top-level orchestrator implementing the `ProductSurface` trait |
| `InboundTurnService` / `DefaultInboundTurnService` | User-message turn submission path |
| `ConversationBindingService` | Resolves external adapter refs → canonical Reborn identifiers |
| `ProductConversationBindingService` | Adapter from product workflow bindings to `ironclaw_conversations` with trusted installation→tenant mapping |
| `StaticProductInstallationResolver` / `ProductInstallationScope` | Host-owned installation registry used by local-dev/tests to select tenant and default agent/project scope |
| `SharedConversationAdmission` | Host-owned shared-conversation admission — answers only "is this shared conversation connected", fail-closed both without a wired port and for unlisted conversations; product workflow checks it before binding a shared conversation. **Declared in `ironclaw_product_contracts::shared_admission`** — product consumes it, `ironclaw_extension_host` implements it |
| `IdempotencyLedger` | Durable action deduplication port |
| `InMemoryIdempotencyLedger` | Local-dev/test ledger with in-flight lease recovery semantics |
| `ProductInboundAction` | Durable ledger record for inbound actions |
| `ProductCommandAdmissionService` | Source/auth-aware admission port that decides whether a typed product command may execute |
| `product-surface command operation` | Reborn-native product command execution port for already-admitted typed commands |
| `ApprovalInteractionService` / `DefaultApprovalInteractionService` | Approval-only product/WebUI boundary for listing redacted pending approval gates and resolving click approve/deny through canonical approval resolver + turn coordinator ports |
| `RunStateApprovalInteractionReadModel` | Canonical read model that returns status-bearing approval gates from scoped approval-request records plus the parked turn-run locator; `ApprovalInteractionService::list_pending` filters those records to pending UI DTOs |
| `AuthInteractionService` / `DefaultAuthInteractionService` | Auth-required product/WebUI boundary for listing redacted pending auth gates and resolving credential/callback/cancel decisions through typed auth-flow manager + turn coordinator ports |
| `ProductSurface` / `RebornServices` | Native WebChat v2 service — stable surface beta WebUI route handlers consume in place of reaching into turn coordination, thread stores, runtime lanes, dispatchers, or capability hosts. Enforces caller ownership of the thread before any turn mutation; projects channel discovery as extension-surface data on the extensions list (typed direction + connect affordance; no separate channel registry); rejects stale or attacker-supplied `gate_ref` on denied/cancelled gate resolutions; routes approval-gate `always: true` resolutions through the approval interaction policy path while keeping generic gate fallback one-shot only |

## Ports that are no longer declared here

WS2 moved the twelve product-side ports this crate declared whose
implementation sits outside it (PROPOSAL §6.1.3) — **ten** implemented by
`ironclaw_extension_host` (the set
`crates/app/ironclaw_architecture_tests/tests/reborn_extension_host_port_inversion.rs`
enumerates and pins as `INVERTED_PORTS`; WS2.1 moved nine and WS2.2 added the
shared-route resolver — since reshaped admission-only as
`SharedConversationAdmission` — once the boundary error made it
declarable) and two by `ironclaw_composition` (`AdminUserService`,
`RebornOperatorToolCatalog`). That test is the enforced inventory; this list is
prose and defers to it.
They now live in `ironclaw_product_contracts` and this crate imports them like
any other consumer — there is deliberately **no re-export** (the port half of
`reborn_product_contract_location_scan.rs` fails on one):

`delivery::{ChannelDeliveryResolver, ResolvedChannelDelivery, DeliveryReplyContextSource}` ·
`account_setup::{AccountConnectionStatusSource, ChannelConnectionNoticePolicy, ExtensionAccountSetupDescriptor, ExtensionAccountSetupError, AccountConnectionStatusError}` ·
`channel_config::ChannelConfigProductService` ·
`views::{RebornViewProvider, RebornViewDescriptor, RebornViewQuery, RebornViewPage}` ·
`command::{ProductCommandContext, CommandActorRoleResolver}` ·
`action::{ProductActionId, ActionFingerprintKey, SourceBindingKey, ProductCommandName, AuthRequestRef, LinkedThreadActionId}` ·
`prompt_source::{ApprovalPromptContextSource, BlockedAuthPromptSource, BlockedAuthPromptRequest}` ·
`lifecycle_service::{LifecycleProductService, LifecycleProductContext, LifecycleProductSurfaceContext}` ·
`admin_users::{AdminUserService, AdminUser*, AdminCreate*}` ·
`operator_tools::{RebornOperatorToolCatalog, RebornOperatorToolInfo}` ·
`shared_admission::{SharedConversationAdmission, SharedConversationAdmissionRequest, ProductConversationRouteKey}` (WS2.2, inverted as `ProductConversationSubjectRouteResolver`; reshaped admission-only when shared-route subjects retired) ·
`error::ProductOperationFailure` (WS2.2 — the boundary error, not a port) ·
`llm_config::{LlmConfigService, ActiveModelReader, + 15 DTOs}` and
`operator_service::{OperatorStatusService, OperatorLogsService, OperatorServiceLifecycleService, + 12 DTOs, normalize_operator_log_context_value}`
(WS5 operator row — implemented by `ironclaw_operator`, except readiness status
which is composition's; pinned by the sibling gate
`reborn_operator_port_inversion.rs`).

**Two of those were the last things `ironclaw_operator` needed from this
crate.** Its `ironclaw_assistant` dependency is gone from the manifest, which is
the point: operator is this crate's *sibling*, not its consumer. If you find
yourself wanting to declare a trait here for the operator to implement, that is
the inversion coming back — declare it in `ironclaw_product_contracts` and add
a row to `INVERTED_PORTS` in that gate.

What this crate kept from the operator move, and why: the frozen view
descriptors (`LLM_CONFIG_VIEW`, `LOGS_VIEW`, `OPERATOR_LOGS_VIEW`) because the
concrete inventory is product's; the fail-closed `Unsupported*` services and the
`Static*` doubles because a default *implementation* is not a contract; the
and the
`RebornOperator*` command-plane envelope that wraps the moved DTOs. The
`LlmConfigServiceError` → `ProductSurfaceError` status table has one home, the
`impl From` in `ironclaw_product_contracts::operator_llm`; call sites here use
`.map_err(ProductSurfaceError::from)` directly, with no product-local alias in
between.

What stayed, and why: the **implementations** (`DeliveryCoordinator`,
`NoReplyContext`, `ExtensionAccountSetupRegistry`, `UnsupportedLifecycleProductService`,
`RejectingAdminUserService`, `UnavailableRebornViewProvider`,
`DirectConversationCommandAdmission`), the frozen **operation inventory** — the
concrete `*_COMMAND`/`*_VIEW`/`*_CAPABILITY` constants §6.1.3 keeps here, which
is why `webui` and `openai_compat` still name this crate at all — the ledger
record and saga (`ProductInboundAction`),
and five ports whose signatures name `ironclaw_auth`/`ironclaw_conversations`
types that a contracts crate may not depend on, or product-declared binding DTOs
— see the residue list in
`crates/app/ironclaw_architecture_tests/tests/reborn_extension_host_port_inversion.rs`.

`ProductSurfaceFailure` is the crate's *internal* workflow error and stays here —
and since WS2.2 that description is **true** rather than aspirational. The
boundary half it used to double as is
`ironclaw_product_contracts::error::ProductOperationFailure`: six variants whose
payloads are a plain `String` or nothing, which is what `ironclaw_extension_host`
constructs and all it ever needed. What stays here is what only this crate
produces and reads — the turn-coordinator variants carrying
`ironclaw_turns::TurnError`, the approval/auth interaction rejection kinds, the
idempotency replay, and the inbound-attachment and policy failures. Two rules:

- **Absorb, never narrow.** `From<ProductOperationFailure>` is total and
  payload-preserving, so `?` over a `product_contracts` port keeps its exact
  discriminant. There is deliberately no conversion the other way: the
  kernel-typed variants have no boundary image, and inventing one would flatten
  them into `Transient`. `auth_continuation.rs` is why — it matches all eight
  `TurnErrorCategory` values structurally and distinguishes two that the
  sanitized projection collapses.
- **`lifecycle_product_surface_error` delegates.** Its six shared arms call the
  contract's projection instead of repeating the status choices; only the
  `Transient` warning is local, because a contracts crate may not log.
  `lifecycle_projection_agrees_with_the_contract_projection_on_shared_variants`
  pins the agreement.

## Dependencies

- `ironclaw_approvals` / `ironclaw_authorization` — canonical approval resolution, the approval request store contract surfaced through the approval resolution/read-model ports, and scoped lease issue ports used by approval interactions
- `ironclaw_auth` — typed product-auth continuation events consumed by the workflow auth bridge
- `ironclaw_conversations` — canonical actor/conversation binding and thread route ownership
- `ironclaw_turns` — turn coordinator, scope, IDs
- `ironclaw_threads` — session thread service contract
- `ironclaw_host_api` — canonical identifiers (TenantId, UserId, etc.)

## Boundary rules

Must NOT depend on: `ironclaw_extension_registry`,
`ironclaw_host_runtime`, `ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_sandbox`,
`ironclaw_network`.

All six are now *enforced* — the `ironclaw_assistant` `BoundaryRule` in
`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs` is the
arbiter, and this list is a copy of it. Until WS5, `ironclaw_extension_registry` was on
this list and **not** on the enforced one, and the crate held the dependency:
`adapter_registry` was its only consumer. Moving that module to its chartered
owners (schema → `ironclaw_extension_contracts::product_adapter_section`,
manifest contract + resolved projection →
`ironclaw_extension_registry::host_api::product_adapter`) dropped the manifest entry and
closed the contradiction PROPOSAL §6.9.1 recorded. A product-tier crate that
needs manifest vocabulary imports the contracts crate, never the registry.

Also not this crate's to own: product adapter transport/rendering logic,
host-runtime execution, capability dispatch, and storage backend details.
Raw secrets, raw host paths, backend error details, and unredacted user
content must not appear in errors, events, snapshots, logs, or docs.

Agent-loop note: product-facing turns enter through workflow services and
canonical turn submission. Do not shortcut directly to `AgentLoopDriver`,
`PlannedDriver`, host runtime services, or loop host factories from adapters or
workflow callers.

Product commands are not turns. Adapters may parse slash syntax at the edge, but
`ProductInboundPayload::Command` must enter the workflow as normalized command
payloads. The source/auth decision belongs to `ProductCommandAdmissionService`;
the source-agnostic command model must not know which product surface produced
the command. Admitted commands dispatch through the product-surface command operation, not
`InboundTurnService`, v1 `SubmissionParser`, v1 command routers, or agent-loop
command handlers.

Approval interactions are click-approval only. Pending approval DTOs must be
redacted, scoped, and derived from canonical run-state/approval records or a
projection read model built from them. Approve/deny decisions must go through
`ApprovalResolutionPort` and `TurnCoordinator`; product/WebUI code must not
directly execute tools or mutate approval stores ad hoc. `AlwaysAllow` is
limited to approval gates backed by the durable persistent approval-policy port;
generic gate fallback remains one-shot only. Persistent approval policy checks
must be performed before approval/resume side effects and must fail closed when
the capability manifest does not allow durable reuse. High-value signing and
attested approvals require a separate service shape with canonical payload
attestation and must not be folded into this redacted click-approval DTO.

Auth interactions are auth-required gates only. Pending auth DTOs must be
redacted, scoped, and derived from typed auth-flow state plus the parked
turn-run locator. Credential/callback completion refs are opaque host-issued
references; raw tokens, OAuth codes, verifier material, provider errors, host
paths, or backend diagnostics must not enter product payloads or projection
DTOs. Resume/cancel decisions must go through `AuthFlowManager` and
`TurnCoordinator`; product/WebUI code must not handle raw credentials, mutate
auth-flow records directly, or resume blocked auth gates without the
`BlockedAuthGate` precondition.

WebUI gate resolution routing should use current run-state first: a
`BlockedApproval` run enters `ApprovalInteractionService`, a `BlockedAuth` run
enters `AuthInteractionService`, and generic fallback is only for non-typed
blocked gates or legacy/replay shapes. Do not let generic WebUI gate handling
resume/cancel auth-blocked runs.
Typed auth/approval interaction services intentionally re-read run-state through
`blocked_gate_state` immediately before resume/cancel side effects. Treat that
second read as a freshness/TOCTOU guard unless a future coordinator returns a
sealed gate grant that can safely replace it.

WebUI-facing service methods must bind browser thread ids through
`SessionThreadService` using a `ThreadScope` derived from the authenticated
caller before accepting messages, streaming events, canceling runs, or resolving
gates. Browser/session metadata is not authority by itself, and send-message
must not implicitly create missing threads.

### Trigger-thread exception

Automation trigger-fired threads are stored by `record_trigger_prompt` with
`owner_user_id = Some(creator_user_id)` — the user who fired the trigger — not
the WebUI caller's session user. The caller-scoped `SessionThreadService` probe
therefore misses them.

When a thread lookup misses under the caller's session scope, `RebornServices`
falls back to `AutomationProductService::resolve_run_thread_scope`. That method
is caller-scoped: it scans only the triggers owned by the authenticated caller
(matched on tenant_id + creator_user_id + agent_id + project_id), so the
authorization check is embedded in the lookup. If a matching trigger run is
found, the service reconstructs a `TurnScope` with:

- `owner_user_id = Some(trigger creator_user_id)` — NOT the session caller
- `agent_id` / `project_id` from the trigger record

and substitutes the trigger creator as the `run_actor` for all downstream turn
operations (timeline, SSE stream, gate resolve, cancel, run-state).

**Visibility for listing and thread authorization are deliberately decoupled.**
The authorization model is caller ownership (tenant + user + agent + project),
not `list_automations` listing eligibility. The default `list_automations`
response excludes `Completed` triggers (soft-completed fire-once triggers) to
keep the active panel uncluttered. But completed triggers' run threads remain
authorized through this resolver — their history is retained user data and must
stay accessible. `resolve_run_thread_scope` does not filter on trigger state.

**No caching.** Every call revalidates automation visibility through the service.
A caller that loses automation visibility mid-session cannot keep accessing the
trigger-owned thread after their access is revoked. Caching the authz result
is explicitly forbidden.

**Backend/timeout errors** from `resolve_run_thread_scope` surface as
`Unavailable` (503, retryable), never as 404. Only `Ok(None)` (thread not in
any caller-visible trigger) produces a 404 response.

WebUI-facing service errors must expose stable, sanitized taxonomy. Keep
`ProductSurfaceErrorCode` aligned with coarse transport/status shape and
`ProductSurfaceErrorKind` aligned with M1-renderable user-safe families such as
validation, duplicate, busy, participant denied, blocked approval/auth/resource,
replay/timeline unavailable, service unavailable, conflict, not found, and
internal. Do not surface backend strings, host paths, provider/runtime details,
raw prompts, tool args, or secrets through the service error payload.

Product adapter bindings must choose `TenantId` only from trusted host
installation configuration, never from inbound adapter payloads. Default
`AgentId`/`ProjectId` for first-contact product turns are also trusted
installation configuration, not external hints, and must be persisted into the
canonical conversation binding on first bind rather than overlaid on every
resolve. Thread hints in subscription requests may narrow to the already
resolved binding only; they are not authority to switch threads or tenants.
Projection/subscription resolution is lookup-only and must not create bindings,
threads, or external-event route reservations.
A run acts as its invoker: a shared conversation is ONE canonical thread
(conversation-keyed — the Slack thread, the Telegram chat/topic) owned by
whoever bound it first and joined by every later paired participant, and each
message inside it runs as its sender — there is no configured subject user.
Admission is presence-based (delivery through the channel's verified ingress
is the membership evidence, consulted through `SharedConversationAdmission`)
and is checked fail-closed on resolve, lookup, and reset: no admission port
wired, or a foreign installation, means rejection — never a fallback
owner.

Outbound delivery orchestration starts only after `ironclaw_outbound` resolves
and validates a communication delivery candidate. `OutboundPolicyService`
remains the authority for reply-target validation and delivery-attempt metadata.
Product workflow may attach trusted product target metadata from conversation
binding and call `ProductAdapter::render_outbound`, but it must not choose a
different reply target, read outbound preferences itself, or render anything
before policy approval. Target metadata resolvers must be lookup-only and keyed
by the sealed validated reply-target binding.

## `reborn_services` module-charter map

`src/reborn_services.rs` is **7,554 lines** — the largest file in this crate —
and carries a live `// arch-exempt: large_file` waiver naming plan #5985. This
map is **not** that split and does not discharge that waiver: PROPOSAL §6.4.15
calls this shape "module-charter work, **not a split**", and §6.9.1 asks for
exactly this — *"the `reborn_services` god-object keeps its freeze ratchet and
gains a module-charter map (the audited ≥ 11 sub-owners)"*. It says which
concern a change belongs to *while the file is still one file*, so the eventual
split inherits a decided seam list instead of an argument.

**The freeze ratchet is a different gate and stays.**
`crates/app/ironclaw_architecture_tests/tests/reborn_service_method_freeze_ratchet.rs`
pins the three-word `ProductSurface` vocabulary (`invoke`/`query`/`stream_events`)
and that no product-local service trait returns. This map governs placement, not
the surface.

**This table is enforced.** `tests/reborn_services_module_charter.rs` asserts
every top-level item in `reborn_services.rs` and its `reborn_services/`
submodules appears in exactly one row, that every name in a row still exists,
and that no name is claimed twice — so a new descriptor or helper fails until it
is given an owner, and a deleted one fails until its entry goes.

**Owners are conceptual, not positional.** The file interleaves capability,
command and view descriptors for a dozen concerns in inventory order, so
contiguous banner-delimited regions would mean moving code — which is the split
this row explicitly is not. The gate is therefore item-granular. `impl` blocks
are not walked: `RebornServices`' inherent methods belong to the owner of
`RebornServices` itself (`dispatch`), and a method that grows into a concern of
its own should become a `reborn_services/` submodule, which the map then charts
by file.

| Sub-owner | Owns | Never contains | Items |
|---|---|---|---|
| `extensions` | Extension listing, install/import/activate/remove, the setup handshake, credential status/submit, onboarding, and the lifecycle projections | Admin *configuration* of an installed extension (that is `admin-config`) and manifest parsing (that is `ironclaw_extension_registry`) | `mod extension_credentials`, `mod extension_onboarding`, `mod extension_setup_credentials`, `mod extensions`, `mod lifecycle_setup`, `mod types`, `EXTENSION_INSTALL_CAPABILITY_ID`, `EXTENSION_INSTALL_CAPABILITY`, `EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID`, `EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY`, `EXTENSION_IMPORT_CAPABILITY_ID`, `EXTENSION_IMPORT_CAPABILITY`, `EXTENSION_ACTIVATE_CAPABILITY_ID`, `EXTENSION_ACTIVATE_CAPABILITY`, `EXTENSION_REMOVE_CAPABILITY_ID`, `EXTENSION_REMOVE_CAPABILITY`, `EXTENSION_SETUP_SUBMIT_CAPABILITY_ID`, `EXTENSION_SETUP_SUBMIT_CAPABILITY`, `StaticChannelConnectionService`, `ExtensionCredentialStatusRequest`, `ExtensionCredentialSubmitRequest`, `ExtensionCredentialSetupService`, `parse_credential_account_id`, `reborn_services/extension_credentials.rs::ExtensionCredentialReadiness`, `reborn_services/extension_credentials.rs::RequirementCredentialReadiness`, `reborn_services/extension_credentials.rs::credential_scope`, `reborn_services/extension_credentials.rs::unique_requirements`, `reborn_services/extension_credentials.rs::presence_readiness_and_missing_scopes`, `reborn_services/extension_credentials.rs::credential_presence_request`, `reborn_services/extension_credentials.rs::credential_status_for_requirement`, `reborn_services/extension_credentials.rs::requirement_readiness_for_status`, `reborn_services/extension_credentials.rs::credential_status_for_requirement_strict`, `reborn_services/extension_credentials.rs::credential_status_request`, `reborn_services/extension_credentials.rs::provider_for_requirement`, `reborn_services/extension_credentials.rs::provider_scopes_for_requirement`, `reborn_services/extension_credentials.rs::is_retryable_status_failure`, `reborn_services/extension_credentials.rs::warn_retryable_status_failure`, `reborn_services/extension_onboarding.rs::ExtensionOnboarding`, `reborn_services/extension_onboarding.rs::for_installed`, `reborn_services/extension_onboarding.rs::for_installed_with_credential_status`, `reborn_services/extension_onboarding.rs::from_lifecycle`, `reborn_services/extension_onboarding.rs::for_summary`, `reborn_services/extension_onboarding.rs::credential_onboarding`, `reborn_services/extension_onboarding.rs::no_credential_onboarding`, `reborn_services/extension_onboarding.rs::activation_instructions`, `reborn_services/extension_onboarding.rs::instructions`, `reborn_services/extension_onboarding.rs::setup_url`, `reborn_services/extension_onboarding.rs::credential_next_step`, `reborn_services/extension_setup_credentials.rs::requirements`, `reborn_services/extension_setup_credentials.rs::project`, `reborn_services/extension_setup_credentials.rs::parse_submit_payload`, `reborn_services/extension_setup_credentials.rs::submit_manual_tokens`, `reborn_services/extension_setup_credentials.rs::submit_manual_token_requirement`, `reborn_services/extension_setup_credentials.rs::setup_projection`, `reborn_services/extension_setup_credentials.rs::credential_prompt`, `reborn_services/extension_setup_credentials.rs::credential_label`, `reborn_services/extension_setup_credentials.rs::SetupSubmitPayload`, `reborn_services/extensions.rs::EXTENSION_READINESS_CONCURRENCY`, `reborn_services/extensions.rs::EXTENSIONS_VIEW`, `reborn_services/extensions.rs::EXTENSION_REGISTRY_VIEW`, `reborn_services/extensions.rs::list_extensions`, `reborn_services/extensions.rs::list_extension_registry`, `reborn_services/extensions.rs::import_extension_capability`, `reborn_services/extensions.rs::execute_lifecycle`, `reborn_services/extensions.rs::lifecycle_surface_context`, `reborn_services/extensions.rs::lifecycle_installed_extensions`, `reborn_services/extensions.rs::lifecycle_extension_infos`, `reborn_services/extensions.rs::registry_entry`, `reborn_services/extensions.rs::is_builtin_host_surface`, `reborn_services/extensions.rs::credential_readiness_for_extension`, `reborn_services/extensions.rs::CallerChannelConnection`, `reborn_services/extensions.rs::CallerChannelMaps`, `reborn_services/extensions.rs::caller_channel_connection`, `reborn_services/extensions.rs::CallerExtensionAuth`, `reborn_services/extensions.rs::caller_extension_auth`, `reborn_services/extensions.rs::extension_info`, `reborn_services/extensions.rs::wire_surfaces`, `reborn_services/extensions.rs::has_external_channel_surface`, `reborn_services/extensions.rs::channel_requires_personal_account`, `reborn_services/extensions.rs::channel_requires_personal_binding`, `reborn_services/extensions.rs::caller_public_state`, `reborn_services/extensions.rs::channel_auth_vendor`, `reborn_services/extensions.rs::vendor_auth_accounts`, `reborn_services/extensions.rs::projected_channel_account`, `reborn_services/lifecycle_setup.rs::EXTENSION_SETUP_VIEW`, `reborn_services/lifecycle_setup.rs::SetupAction`, `reborn_services/lifecycle_setup.rs::setup_extension_view`, `reborn_services/lifecycle_setup.rs::submit_extension_setup_capability`, `reborn_services/lifecycle_setup.rs::setup_extension`, `reborn_services/lifecycle_setup.rs::project_package`, `reborn_services/lifecycle_setup.rs::project_package_after_mutation`, `reborn_services/lifecycle_setup.rs::channel_field_status`, `reborn_services/lifecycle_setup.rs::route_channel_config_values`, `reborn_services/lifecycle_setup.rs::setup_extension_response`, `reborn_services/lifecycle_setup.rs::setup_public_phase`, `reborn_services/lifecycle_setup.rs::setup_action`, `reborn_services/lifecycle_setup.rs::parse_hosted_mcp_auth_selection`, `reborn_services/lifecycle_setup.rs::validation_error`, `reborn_services/lifecycle_setup.rs::map_lifecycle_error`, `reborn_services/types.rs::RebornExtensionListResponse`, `reborn_services/types.rs::RebornVendorAuthAccounts`, `reborn_services/types.rs::RebornAuthAccount`, `reborn_services/types.rs::RebornExtensionInfo` |
| `admin-config` | Operator configuration keys, the global auto-approve setting, and the tool-permission model (effective state, hard floors, persistent user policy) | Operator *service* control or diagnostics — those are `operator` | `mod admin_configuration`, `mod operator_config_views`, `AUTO_APPROVE_CONFIG_KEY`, `TOOL_CONFIG_PREFIX`, `OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID`, `OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY`, `OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID`, `OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY`, `OPERATOR_CONFIG_SET_KEY_COMMAND_ID`, `OPERATOR_CONFIG_SET_KEY_COMMAND`, `GLOBAL_AUTO_APPROVE_VIEW`, `RebornOperatorApprovalConfig`, `operator_config_not_wired_response`, `operator_config_unknown_key_error`, `operator_config_invalid_value`, `operator_config_store_error`, `operator_config_capability_forbidden`, `product_view_requires_operator_config`, `operator_config_auto_approve_activity_id`, `operator_config_mutation_succeeded`, `auto_approve_config_entry`, `find_operator_tool`, `tool_config_entry`, `tool_config_entry_with_context`, `OperatorToolPermissionContext`, `operator_tool_permission_context`, `effective_tool_permission`, `persistent_user_policy_active`, `persistent_user_policy_key`, `operator_tool_permission_scope`, `tool_permission_locked`, `hard_floor_tool`, `default_tool_permission_state`, `tool_permission_state_wire`, `ToolPermissionUpdate`, `parse_tool_permission_state`, `apply_tool_permission_state`, `operator_config_surface_not_wired_diagnostic`, `operator_config_validation_diagnostics`, `operator_config_key_diagnostic`, `reborn_services/admin_configuration.rs::ADMIN_CONFIGURATION_VIEW`, `reborn_services/admin_configuration.rs::ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID`, `reborn_services/admin_configuration.rs::ADMIN_CONFIGURATION_REPLACE_CAPABILITY`, `reborn_services/admin_configuration.rs::RebornAdminConfigurationListResponse`, `reborn_services/admin_configuration.rs::RebornAdminConfigurationGroup`, `reborn_services/admin_configuration.rs::RebornAdminConfigurationField`, `reborn_services/admin_configuration.rs::RebornAdminConfigurationUse`, `reborn_services/operator_config_views.rs::OPERATOR_CONFIG_LIST_VIEW`, `reborn_services/operator_config_views.rs::OPERATOR_CONFIG_KEY_VIEW`, `reborn_services/operator_config_views.rs::OPERATOR_CONFIG_VALIDATE_VIEW`, `reborn_services/operator_config_views.rs::OperatorConfigKeyViewParams` |
| `operator` | Operator status, logs, service lifecycle, first-run setup validation, and the doctor/diagnostics projections | Configuration values — those are `admin-config` | `mod log_views`, `mod operator_command_views`, `OPERATOR_SETUP_RUN_CAPABILITY_ID`, `OPERATOR_SETUP_RUN_CAPABILITY`, `OPERATOR_SERVICE_LIFECYCLE_COMMAND_ID`, `OPERATOR_SERVICE_LIFECYCLE_COMMAND`, `OPERATOR_LOGS_DEFAULT_LIMIT`, `OPERATOR_LOGS_MAX_LIMIT`, `OPERATOR_LOGS_CURSOR_MAX_BYTES`, `OPERATOR_LOGS_TARGET_MAX_BYTES`, `StaticOperatorStatusService`, `UnsupportedOperatorStatusService`, `UnsupportedOperatorLogsService`, `UnsupportedOperatorServiceLifecycleService`, `operator_setup_validation_error`, `operator_setup_diagnostic`, `OPERATOR_SETUP_PROFILE_ID_MAX_BYTES`, `OPERATOR_SETUP_WEBUI_TOKEN_MIN_BYTES`, `OPERATOR_SETUP_WEBUI_TOKEN_MAX_BYTES`, `OPERATOR_SETUP_REDACTED_SECRET_SENTINEL`, `validate_operator_setup_profile_id`, `validate_operator_setup_webui_access_token`, `reject_unwired_operator_setup_host_mutation`, `OperatorSetupHostState`, `setup_response_from_llm_snapshot`, `operator_doctor_status_diagnostic`, `operator_doctor_status_response`, `operator_doctor_status_check`, `operator_doctor_status_reason_code`, `is_operator_doctor_reason_code_component`, `operator_doctor_status_text`, `operator_doctor_status_text_needs_redaction`, `operator_doctor_setup_unavailable_diagnostic`, `operator_doctor_status_unavailable_diagnostic`, `operator_diagnostics_surface_status`, `operator_surface_unavailable`, `validate_log_query_modes`, `bounded_operator_logs_query`, `bounded_log_query`, `bounded_operator_logs_string`, `bounded_operator_logs_context_string`, `reborn_services/log_views.rs::LOGS_VIEW`, `reborn_services/log_views.rs::OPERATOR_LOGS_VIEW`, `reborn_services/operator_command_views.rs::OPERATOR_DIAGNOSTICS_VIEW`, `reborn_services/operator_command_views.rs::OPERATOR_STATUS_VIEW`, `reborn_services/operator_command_views.rs::OPERATOR_SETUP_VIEW` |
| `inspector` | Operator-only reads of bounded, process-local run diagnostics and their resumable update batches | Normal product projections, durable events, or diagnostic capture policy — this owner only authorizes and reads the shared store | `mod inspector`, `reborn_services/inspector.rs::diagnostic_scope`, `reborn_services/inspector.rs::store_error`, `reborn_services/inspector.rs::snapshot`, `reborn_services/inspector.rs::prompt`, `reborn_services/inspector.rs::tool`, `reborn_services/inspector.rs::updates` |
| `automations` | Automation listing and lifecycle (pause/resume/rename/delete), automation name validation, trigger-thread recognition, and the notification-approval candidate scan | Trigger evaluation or scheduling — that is `ironclaw_triggers` | `AUTOMATION_PAUSE_CAPABILITY_ID`, `AUTOMATION_PAUSE_CAPABILITY`, `AUTOMATION_RESUME_CAPABILITY_ID`, `AUTOMATION_RESUME_CAPABILITY`, `AUTOMATION_RENAME_CAPABILITY_ID`, `AUTOMATION_RENAME_CAPABILITY`, `AUTOMATION_DELETE_CAPABILITY_ID`, `AUTOMATION_DELETE_CAPABILITY`, `AUTOMATION_PAUSE_COMMAND_ID`, `AUTOMATION_PAUSE_COMMAND`, `AUTOMATION_RESUME_COMMAND_ID`, `AUTOMATION_RESUME_COMMAND`, `AUTOMATION_RENAME_COMMAND_ID`, `AUTOMATION_RENAME_COMMAND`, `AUTOMATION_DELETE_COMMAND_ID`, `AUTOMATION_DELETE_COMMAND`, `AUTOMATIONS_VIEW`, `AutomationListRequest`, `TriggerRunThreadScope`, `AutomationNotificationTitle`, `AutomationApprovalThreadCandidate`, `AutomationProductService`, `UnsupportedAutomationProductService`, `automation_unavailable`, `is_automation_trigger_thread`, `AUTOMATION_LIST_DEFAULT_PAGE_SIZE`, `AUTOMATION_LIST_MAX_PAGE_SIZE`, `AUTOMATION_RUN_HISTORY_DEFAULT_PAGE_SIZE`, `AUTOMATION_RUN_HISTORY_MAX_PAGE_SIZE`, `NOTIFICATION_APPROVAL_AUTOMATION_LIMIT`, `NOTIFICATION_APPROVAL_RUN_LIMIT`, `NOTIFICATION_APPROVAL_CANDIDATE_LIMIT`, `NOTIFICATION_APPROVAL_QUERY_TIMEOUT`, `clamp_automation_list_limit`, `clamp_automation_run_limit`, `parse_automation_name`, `automation_name_validation_code`, `automation_name_validation_error`, `notification_approval_timeout_error` |
| `suggestions` | Durable, per-user suggestion generation and lifecycle wiring | Suggestion prompt policy and durable storage implementation — those are `suggestions.rs` and `suggestions_store.rs` | `SuggestionsServices` |
| `outbound` | Outbound delivery targets, notification preferences, and the outbound delivery capability surface the model calls | Delivery itself — the host-owned `DeliveryCoordinator` owns that | `mod outbound_delivery_capability_surface`, `mod outbound_preferences`, `mod outbound_views`, `NOTIFICATION_CHANNELS_SET_COMMAND`, `NOTIFICATION_CHANNELS_SET_COMMAND_ID`, `OutboundPreferencesProductService`, `UnsupportedOutboundPreferencesProductService`, `outbound_preferences_unavailable`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_DELIVERY_SYNTHETIC_PROVIDER_ID`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID`, `reborn_services/outbound_delivery_capability_surface.rs::NOTIFICATION_CHANNELS_SET_MAX_ITEMS`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_NOTIFICATION_CHANNELS_SET_PROVIDER_TOOL_NAME`, `reborn_services/outbound_delivery_capability_surface.rs::OUTBOUND_NOTIFICATION_CHANNELS_SET_DESCRIPTION`, `reborn_services/outbound_delivery_capability_surface.rs::outbound_delivery_synthetic_provider`, `reborn_services/outbound_delivery_capability_surface.rs::notification_channels_set_operator_tool_info`, `reborn_services/outbound_delivery_capability_surface.rs::OutboundDeliveryTargetsListInput`, `reborn_services/outbound_delivery_capability_surface.rs::NotificationChannelsSetInput`, `reborn_services/outbound_delivery_capability_surface.rs::OutboundDeliveryCapabilityInputError`, `reborn_services/outbound_delivery_capability_surface.rs::list_outbound_delivery_targets_for_model`, `reborn_services/outbound_delivery_capability_surface.rs::set_notification_channels_for_model`, `reborn_services/outbound_delivery_capability_surface.rs::outbound_delivery_targets_list_input_schema`, `reborn_services/outbound_delivery_capability_surface.rs::notification_channels_set_input_schema`, `reborn_services/outbound_delivery_capability_surface.rs::parse_outbound_delivery_targets_list_input`, `reborn_services/outbound_delivery_capability_surface.rs::parse_notification_channels_set_input`, `reborn_services/outbound_delivery_capability_surface.rs::input_object`, `reborn_services/outbound_preferences.rs::RebornOutboundPreferencesService`, `reborn_services/outbound_preferences.rs::target_scope`, `reborn_services/outbound_preferences.rs::reborn_target_id_from_outbound`, `reborn_services/outbound_preferences.rs::mod notification_channels`, `reborn_services/outbound_preferences.rs::reborn_summary_from_outbound`, `reborn_services/outbound_preferences.rs::reborn_capabilities_from_outbound`, `reborn_services/outbound_preferences.rs::outbound_target_projection_error`, `reborn_services/outbound_preferences.rs::map_outbound_repository_error`, `reborn_services/outbound_views.rs::NOTIFICATION_CHANNELS_VIEW`, `reborn_services/outbound_views.rs::OUTBOUND_DELIVERY_TARGETS_VIEW` |
| `notification-setup` | The generic per-channel notification-setup surface (§8): status/enable/disable over host-owned delivery registrations, including pre-storage admission against manifest-declared egress hosts and publication of bounded client bootstrap data | Channel-specific registration parsing or push protocol mechanics; push delivery stays with the delivery coordinator | `mod notification_setup`, `reborn_services/notification_setup.rs::ChannelNotificationSetupService`, `reborn_services/notification_setup.rs::DeliveryClientBootstrap`, `reborn_services/notification_setup.rs::DeliveryClientBootstrapError`, `reborn_services/notification_setup.rs::NoDeliveryClientBootstrap`, `reborn_services/notification_setup.rs::UnsupportedChannelNotificationSetupService`, `reborn_services/notification_setup.rs::RegistrationChannelNotificationSetupService`, `reborn_services/notification_setup.rs::EnrollmentSubmission`, `reborn_services/notification_setup.rs::invalid_payload`, `reborn_services/notification_setup.rs::notification_setup_unavailable`, `reborn_services/notification_setup.rs::map_registration_error` |
| `threads` | Thread lifecycle and access resolution, the thread-list and timeline reads, their page-size clamps and cursor codec, and per-thread operation locking | Turn submission or run control (that is `runs`), and artifact export (that is `run-artifact`) | `THREAD_DELETE_CAPABILITY_ID`, `THREAD_DELETE_CAPABILITY`, `CREATE_THREAD_COMMAND_ID`, `CREATE_THREAD_COMMAND`, `THREADS_VIEW`, `TIMELINE_VIEW`, `ThreadOperationLocks`, `ResolvedThreadAccess`, `map_ownership_probe_error`, `thread_scope_from_turn_scope`, `parse_thread_id_field`, `thread_operation_key`, `TIMELINE_DEFAULT_PAGE_SIZE`, `TIMELINE_MAX_PAGE_SIZE`, `TIMELINE_MAX_SUMMARY_ARTIFACTS`, `THREAD_LIST_DEFAULT_PAGE_SIZE`, `THREAD_LIST_MAX_PAGE_SIZE`, `THREAD_LIST_FILTER_MIN_FETCH_SIZE`, `THREAD_LIST_FILTER_MAX_PAGES`, `clamp_timeline_limit`, `clamp_thread_list_limit`, `TimelineCursor`, `parse_timeline_cursor`, `serialize_timeline_cursor`, `paginate_timeline_messages`, `cap_summary_artifacts`, `map_timeline_probe_error`, `map_thread_error`, `delete_thread_busy`, `create_thread_metadata_json`, `generated_thread_id`, `reborn_services/types.rs::RebornCreateThreadResponse`, `reborn_services/types.rs::RebornTimelineResponse`, `reborn_services/types.rs::RebornListThreadsResponse` |
| `runs` | Turn submission, cancel, retry, run state, steering admission, and the WebUI accepted-message/replay idempotency plumbing that feeds them | Gate resolution (that is `gates`) and stream transport (that is `projections`) | `SUBMIT_TURN_COMMAND_ID`, `SUBMIT_TURN_COMMAND`, `SESSION_SURFACE_ADAPTER_ID`, `SESSION_ACTOR_KIND`, `SessionModelSelectionPolicy`, `session_inbound_request`, `session_rejection_error`, `CANCEL_RUN_COMMAND_ID`, `CANCEL_RUN_COMMAND`, `RETRY_RUN_COMMAND_ID`, `RETRY_RUN_COMMAND`, `NOTICE_BUSY_GENERIC`, `describe_turn_status`, `rejected_busy_notice`, `parse_run_id_field`, `parse_persisted_turn_run_id`, `map_turn_error`, `reborn_services/types.rs::reborn_cancel_run_response`, `reborn_services/types.rs::reborn_retry_run_response`, `reborn_services/types.rs::RebornGetRunStateResponse` |
| `projects` | Project CRUD, project membership, and project-scoped filesystem reads | Workspace-wide mount browsing — that is `workspace-fs` | `mod project_fs`, `mod projects`, `PROJECT_UPDATE_CAPABILITY_ID`, `PROJECT_UPDATE_CAPABILITY`, `PROJECT_DELETE_CAPABILITY_ID`, `PROJECT_DELETE_CAPABILITY`, `PROJECT_MEMBER_ADD_CAPABILITY_ID`, `PROJECT_MEMBER_ADD_CAPABILITY`, `PROJECT_MEMBER_UPDATE_CAPABILITY_ID`, `PROJECT_MEMBER_UPDATE_CAPABILITY`, `PROJECT_MEMBER_REMOVE_CAPABILITY_ID`, `PROJECT_MEMBER_REMOVE_CAPABILITY`, `PROJECT_CREATE_COMMAND_ID`, `PROJECT_CREATE_COMMAND`, `PROJECT_FS_READ_COMMAND_ID`, `PROJECT_FS_READ_COMMAND`, `PROJECT_FS_LIST_VIEW`, `PROJECT_FS_STAT_VIEW`, `PROJECTS_VIEW`, `PROJECT_VIEW`, `PROJECT_MEMBERS_VIEW`, `map_project_fs_error`, `project_caller`, `map_project_service_error`, `reborn_services/project_fs.rs::ProjectFilesystemReader`, |
| `llm-admin` | LLM provider upsert/delete, active-model selection, user model policy/catalog, connection tests, model listing, the provider login commands, and base-URL validation | The provider implementations — those are `ironclaw_llm`; transport-consumed policy/catalog descriptors live in `ironclaw_product_contracts::operator_llm` | `mod llm_config`, `LLM_PROVIDER_UPSERT_CAPABILITY_ID`, `LLM_PROVIDER_UPSERT_CAPABILITY`, `LLM_PROVIDER_DELETE_CAPABILITY_ID`, `LLM_PROVIDER_DELETE_CAPABILITY`, `LLM_ACTIVE_SET_CAPABILITY_ID`, `LLM_ACTIVE_SET_CAPABILITY`, `LLM_TEST_CONNECTION_COMMAND_ID`, `LLM_TEST_CONNECTION_COMMAND`, `LLM_LIST_MODELS_COMMAND_ID`, `LLM_LIST_MODELS_COMMAND`, `LLM_NEARAI_LOGIN_COMMAND_ID`, `LLM_NEARAI_LOGIN_COMMAND`, `LLM_NEARAI_WALLET_LOGIN_COMMAND_ID`, `LLM_NEARAI_WALLET_LOGIN_COMMAND`, `LLM_CODEX_LOGIN_COMMAND_ID`, `LLM_CODEX_LOGIN_COMMAND`, `LLM_BASE_URL_MAX_BYTES`, `validate_llm_base_url`, `forbidden_llm_base_url_ip`, `forbidden_llm_base_url_ipv4`, `forbidden_llm_base_url_ipv6`, `reborn_services/llm_config.rs::LLM_CONFIG_VIEW`, `reborn_services/llm_config.rs::llm_config_unavailable`, `reborn_services/llm_config.rs::llm_config_input_error` |
| `commands` | The product command grammar surface: listing declared commands, executing one, and the capability handler dispatch table | A command's *effect* — each handler delegates to the concern that owns it | `mod product_capability_handlers`, `mod product_commands`, `PRODUCT_COMMAND_LIST_COMMAND_ID`, `PRODUCT_COMMAND_LIST_COMMAND`, `PRODUCT_COMMAND_EXECUTE_COMMAND_ID`, `PRODUCT_COMMAND_EXECUTE_COMMAND`, `command_result_field`, `model_command_view`, `user_model_preference_command_view`, `idle_status_command_view`, `nothing_to_stop_command_view`, `new_conversation_started_view`, `reborn_services/types.rs::RebornProductCommandEffect`, `product_command_input`, `reborn_services/product_capability_handlers.rs::ProductCommandHandler`, `reborn_services/product_capability_handlers.rs::command_output`, `reborn_services/product_capability_handlers.rs::ProductCapabilityHandler`, `reborn_services/product_commands.rs::lifecycle_command_title`, `reborn_services/product_commands.rs::lifecycle_command_view`, `reborn_services/product_commands.rs::lifecycle_rows_view`, `reborn_services/product_commands.rs::lifecycle_confirmation_view`, `reborn_services/product_commands.rs::package_ref_fields`, `reborn_services/product_commands.rs::package_kind_label`, `reborn_services/product_commands.rs::blocker_line`, `reborn_services/product_commands.rs::capability_lines`, `reborn_services/product_commands.rs::extension_row`, `reborn_services/product_commands.rs::skill_row`, `reborn_services/product_commands.rs::yes_no`, `reborn_services/types.rs::RebornExecuteProductCommandResponse` |
| `admin-users` | Admin user CRUD, role/status, per-user secrets, and last-admin protection | Authorization of the caller — that is `dispatch`'s view gate | `mod admin_users`, `ADMIN_USER_UPDATE_CAPABILITY_ID`, `ADMIN_USER_UPDATE_CAPABILITY`, `ADMIN_USER_SET_STATUS_CAPABILITY_ID`, `ADMIN_USER_SET_STATUS_CAPABILITY`, `ADMIN_USER_SET_ROLE_CAPABILITY_ID`, `ADMIN_USER_SET_ROLE_CAPABILITY`, `ADMIN_USER_DELETE_CAPABILITY_ID`, `ADMIN_USER_DELETE_CAPABILITY`, `ADMIN_USER_PUT_SECRET_CAPABILITY_ID`, `ADMIN_USER_PUT_SECRET_CAPABILITY`, `ADMIN_USER_DELETE_SECRET_CAPABILITY_ID`, `ADMIN_USER_DELETE_SECRET_CAPABILITY`, `ADMIN_USER_CREATE_COMMAND_ID`, `ADMIN_USER_CREATE_COMMAND`, `ADMIN_USER_DELETE_SECRET_COMMAND_ID`, `ADMIN_USER_DELETE_SECRET_COMMAND`, `ADMIN_USERS_VIEW`, `ADMIN_USER_VIEW`, `ADMIN_USER_SECRETS_VIEW`, `map_admin_user_error`, `last_admin_error`, `reborn_services/admin_users.rs::RejectingAdminUserService` |
| `dispatch` | The `RebornServices` object itself, the caller/scope plumbing every concern shares (`ProductAgentBoundCaller`, resource scope, secret handles), the `ProductSurfaceError` shaping helpers, and the view-authorization gate | A concern's own logic — an item belongs here only when **more than one** sub-owner calls it | `mod views`, `ProductAgentBoundCaller`, `caller_resource_scope`, `product_view_forbidden`, `authorize_product_view`, `ProductCapabilityInvoker`, `UnavailableProductCapabilityInvoker`, `RebornServices`, `product_capability_input_error`, `product_secret_handle`, `segment`, `map_adapter_error`, `code_for_status`, `kind_for_surface_rejection`, `truncate_utf8_to_bytes`, `product_agent_bound_caller_from_webui`, `reborn_services/views.rs::EmptyViewParams`, `reborn_services/views.rs::parse_empty_view_params`, `reborn_services/views.rs::required_string_view_param`, `reborn_services/views.rs::view_page`, `reborn_services/views.rs::view_page_with_cursor`, `reborn_services/views.rs::UnavailableRebornViewProvider` |
| `run-artifact` | Run and thread artifact export views | Live run state — that is `runs` | `mod run_artifact`, `mod thread_artifact`, `mod timings_source`, `ADMIN_THREAD_SCRAPE_THREADS_VIEW`, `ADMIN_THREAD_SCRAPE_ARTIFACT_VIEW`, `ADMIN_THREAD_SCRAPE_RUN_ARTIFACT_VIEW`, `reborn_services/run_artifact.rs::RUN_ARTIFACT_SCHEMA`, `reborn_services/run_artifact.rs::RUN_ARTIFACT_VIEW`, `reborn_services/run_artifact.rs::ARTIFACT_REDACTION_PIPELINE`, `reborn_services/run_artifact.rs::RebornRunArtifact`, `reborn_services/run_artifact.rs::RunArtifactMessage`, `reborn_services/run_artifact.rs::RunArtifactToolCall`, `reborn_services/run_artifact.rs::RunArtifactLogs`, `reborn_services/run_artifact.rs::RunArtifactRedaction`, `reborn_services/run_artifact.rs::mod timings`, `reborn_services/run_artifact.rs::context_messages_by_id`, `reborn_services/run_artifact.rs::artifact_messages`, `reborn_services/run_artifact.rs::redact_text`, `reborn_services/run_artifact.rs::redact_json`, `reborn_services/run_artifact.rs::redact_json_strings`, `reborn_services/thread_artifact.rs::THREAD_ARTIFACT_SCHEMA`, `reborn_services/thread_artifact.rs::THREAD_ARTIFACT_MAX_MESSAGES`, `reborn_services/thread_artifact.rs::THREAD_ARTIFACT_MAX_STORED_BYTES`, `reborn_services/thread_artifact.rs::THREAD_ARTIFACT_MAX_SERIALIZED_BYTES`, `reborn_services/thread_artifact.rs::THREAD_ARTIFACT_VIEW`, `reborn_services/thread_artifact.rs::RebornThreadArtifact`, `reborn_services/thread_artifact.rs::RunArtifactRunTimings`, `reborn_services/thread_artifact.rs::group_messages_by_run`, `reborn_services/thread_artifact.rs::thread_artifact_too_large`, `reborn_services/timings_source.rs::derive_wall_clock_ms` |
| `skills` | Skill install/update/remove, search and content reads, auto-activation settings, and the activation recorder/clearer ports | Skill *selection* — that is `ironclaw_skills` | `SkillActivationRecorder`, `SkillActivationClearer`, `SKILL_INSTALL_CAPABILITY_ID`, `SKILL_INSTALL_CAPABILITY`, `SKILL_UPDATE_CAPABILITY_ID`, `SKILL_UPDATE_CAPABILITY`, `SKILL_REMOVE_CAPABILITY_ID`, `SKILL_REMOVE_CAPABILITY`, `SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID`, `SKILL_AUTO_ACTIVATE_SET_CAPABILITY`, `SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID`, `SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY`, `SKILLS_VIEW`, `SKILL_SEARCH_VIEW`, `SKILL_CONTENT_VIEW`, `SkillsProductService`, `UnsupportedSkillsProductService` |
| `gates` | Approval and auth gate routing: which resolver a `gate_ref` reaches, stale/attacker-supplied ref rejection, and the blocked-gate notices | The approval or auth *policy* — that lives in `approval_interaction` / `auth_interaction`, not in this module | `mod approval_settings`, `RESOLVE_GATE_COMMAND_ID`, `RESOLVE_GATE_COMMAND`, `NOTICE_BLOCKED_APPROVAL`, `NOTICE_BLOCKED_AUTH`, `GateResolutionRoute`, `validate_current_gate_ref`, `participant_denied`, `assert_generic_run_parked_on_gate`, `reject_generic_auth_gate_resolution`, `persistent_approval_unavailable`, `blocked_approval_unavailable`, `blocked_authentication_unavailable`, `map_auth_interaction_error`, `reborn_services/types.rs::reborn_resume_gate_response` |
| `traces` | Trace credits, trace holds, and the trace account login link | Trace *content* — that is `ironclaw_trace_commons` | `mod trace_credits`, `TRACE_ACCOUNT_LOGIN_LINK_COMMAND_ID`, `TRACE_ACCOUNT_LOGIN_LINK_COMMAND`, `TRACE_HOLD_AUTHORIZE_COMMAND_ID`, `TRACE_HOLD_AUTHORIZE_COMMAND`, `reborn_services/trace_credits.rs::TRACE_CREDITS_VIEW`, `reborn_services/trace_credits.rs::TRACE_ACCOUNT_TRACES_VIEW`, `reborn_services/trace_credits.rs::TRACE_CREDITS_NOTE`, `reborn_services/trace_credits.rs::AccountLoginLinkMintError`, `reborn_services/trace_credits.rs::account_login_link_for_user`, `reborn_services/trace_credits.rs::AccountTracesError`, `reborn_services/trace_credits.rs::account_traces_for_user`, `reborn_services/trace_credits.rs::authorize_trace_hold_for_user`, `reborn_services/trace_credits.rs::local_trace_credits_for_user` |
| `workspace-fs` | Mount-catalog and workspace filesystem reads, attachment reads, and the browse scope they resolve against | Project-scoped reads — those are `projects` | `mod fs_browse`, `FS_READ_COMMAND_ID`, `FS_READ_COMMAND`, `ATTACHMENT_READ_COMMAND_ID`, `ATTACHMENT_READ_COMMAND`, `FS_MOUNTS_VIEW`, `FS_LIST_VIEW`, `FS_STAT_VIEW`, `caller_browse_scope`, `reborn_services/fs_browse.rs::FilesystemBrowseReader` |
| `projections` | Product-surface event subscription, stream request/response codecs, and the first-event and access-revalidation timing | What a stream carries — a projection replays what the surface already produced | `PRODUCT_STREAM_FIRST_EVENT_WAIT`, `PRODUCT_STREAM_ACCESS_REVALIDATION_INTERVAL`, `open_product_surface_event_subscription`, `decode_product_surface_stream_request`, `encode_product_surface_stream_response`, `map_projection_error` |
| `ironhub` | The IronHub install-link handshake | Extension installation itself — that is `extensions` | `mod ironhub_link`, `reborn_services/ironhub_link.rs::ironhub_link_unavailable`, `reborn_services/ironhub_link.rs::map_ironhub_link_error` |

## Test support

Enable `test-support` feature for in-memory fakes:
- `FakeConversationBindingService`
- `FakeIdempotencyLedger`
- `FakeInboundTurnService`

## Validation

- Fast local check: `cargo test -p ironclaw_assistant`
- Lint check: `cargo clippy -p ironclaw_assistant --all-targets -- -D warnings` (the self dev-dependency unifies `test-support` on, so this lints the feature-on shape) plus the production shape with dev-dependencies off: `cargo clippy -p ironclaw_assistant -- -D warnings` (mirrors the merge-queue `--lib --bins` lane; the #7119 unused-import class is only visible here)
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold`; run the full `cargo test -p ironclaw_architecture_tests` sweep when a change touches more than dependency edges (contracts, inventories, pinned guidance)

## Agent notes

- Keep product adapters thin; adapter-specific code should not reimplement workflow ownership from this crate.
- User-message acceptance must persist canonical thread content through `ironclaw_threads::SessionThreadService` before turn submission.
- Do not return a successful product acknowledgement unless the inbound action has a durable terminal ledger outcome.
- Prefer caller-level tests when helpers gate ledger settlement, thread mutation, turn submission, gate resolution, projection access, or other side effects.
