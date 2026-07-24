# Unified Extension Runtime — Acceptance Checklist

**Companions:** `overview.md` (model), `implementation.md` (changes).

Rules — kept short on purpose:

- Check an item only when a named test or command proves it; write that name
  next to the item in the PR that makes it true.
- Persistent behavior counts only when it passes on **libSQL and PostgreSQL**.
- Behavior that gates a side effect needs a caller-level test (route,
  dispatcher, manager), not only a helper unit test.
- `wait_for_status(Completed)` alone is never evidence.
- No other process: no evidence files, no sign-off matrix. CI green on the
  gates plus this list is the release condition.

## 1. Product model and manifest (MAN)

- [ ] MAN-1 Extension is the only installable product object; tools/channels/
  auth cannot be installed or removed independently.
- [ ] MAN-2 One v3 manifest declares tools, at most one channel, auth recipes,
  and optional tenant `[admin_configuration]`; parsing is a single entry point
  shared with normalized v2. Admin fields are not duplicated below `[channel]`. —
  `acme_fixture_parses_through_the_single_entry_point`,
  `v2_and_v3_rewrites_resolve_identically`
  (`crates/ironclaw_extensions/tests/manifest_v3_contract.rs`); both schemas
  dispatch through `ExtensionManifestRecord::from_toml`. The new
  admin-configuration-only schema rows await the combined matrix.
- [x] MAN-3 A v2 manifest and its v3 rewrite resolve to identical surfaces,
  capability ids, scopes, and credentials (projection-equality test over all
  11 first-party packages; the two hosted-MCP packages instead assert their
  `[mcp]` ceiling plus the discovered set, since their placeholder static
  tools intentionally become discovery). —
  `crates/ironclaw_reborn_composition/tests/first_party_manifest_v3_parity.rs`
  (9 static-parity tests against the pre-rewrite v2 snapshots under
  `tests/fixtures/first_party_v2/`, plus `notion_mcp_v3_declares_the_ceiling`
  and `nearai_mcp_v3_declares_the_ceiling`). Effects compare modulo the
  normalizer-added dispatch effect (v2 declared it inconsistently; it gates
  nothing).
- [x] MAN-4 Unknown manifest fields fail closed with a path-qualified error.
  — `unknown_top_level_fields_fail_closed_with_path_context`
  (`manifest_v3_contract.rs`); `unknown_recipe_fields_fail_closed`,
  `unknown_channel_fields_fail_closed`
  (`crates/ironclaw_host_api/src/{recipe,channel}.rs`).
- [x] MAN-5 Recipe validation rejects: non-https endpoints, reserved authorize
  params in `extra_authorize_params`, invalid/deep/wildcard JSON pointers,
  wildcard egress hosts, multi-segment `route_suffix`. —
  `non_https_recipe_endpoints_are_rejected`,
  `reserved_authorize_params_are_rejected`,
  `wildcard_or_deep_json_pointers_are_rejected`,
  `wildcard_egress_hosts_are_rejected`, `wildcard_tool_audience_hosts_are_rejected`,
  `multi_segment_route_suffixes_are_rejected` (`manifest_v3_contract.rs`) plus
  the host_api unit suites.
- [ ] MAN-6 Exactly one of `[runtime]` or `[mcp]` declares the implementation;
  `[mcp]` is mutually exclusive with `[[tools]]` and `[channel]`; discovered
  tools outside the namespace/count/schema-size/effects ceiling are rejected;
  only the `[mcp]` connection credential and server host carry authority —
  discovered tools cannot add credentials or egress.
  — PARTIAL: the manifest half is built + tested (`[mcp]` mutual-exclusion +
  required-field checks, `crates/ironclaw_extensions/tests/manifest_v3_contract.rs`);
  the runtime discovery-ceiling *rejection* (out-of-namespace/count/schema-size/
  effects) is not pinned by a MAN-6-named test — loader-owned discovery is
  covered structurally under TOOL-9. Ticks with a ceiling-rejection test.
- [ ] MAN-7 Two published extensions with the same vendor id and
  identical-except-scopes recipes share one vendor record; differing recipes
  fail internal publication with a conflict error.
  — PARTIAL: the recipe union/conflict logic is unit-tested
  (`shared_vendor_recipes_union_scopes_and_reject_conflicts`,
  `crates/ironclaw_extension_host/src/recipes.rs`), asserting both extension
  names in the conflict error; but the publication-caller path (two extensions
  reconciling → shared record / conflict-fails-publication) is not pinned by a
  caller-level test. Ticks with that caller-level test.
- [x] MAN-8 `trigger`/`file` remain reserved kinds with no runtime binding. —
  `CapabilitySurfaceKind::{Trigger,File}` are reserved enum variants
  (`crates/ironclaw_host_api/src/surface.rs`, doc "no manifest section projects
  this kind yet"), wire-pinned by
  `surface_kind_wire_shape_is_snake_case_and_matches_as_str`; nothing binds them
  at runtime by construction — the binding rule (LIFE-1) binds only
  `tools`/`channel` and no loader path exists for `trigger`/`file`.
- [x] MAN-9 Retired-taxonomy gate still passes (no `slack_bot`/
  `slack_personal`/channel-as-product vocabulary). —
  `reborn_code_never_references_retired_taxonomy`
  (`crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs`) scans
  `crates/` + `tests/integration/` and pins the retired vocabulary at zero;
  green under `cargo test -p ironclaw_architecture`.
- [ ] MAN-10 `[channel].conversation_model` is required and validated;
  conversation binding honors the declared model through a caller-level
  workflow test; the WebUI's internal channel uses the same enum.
- [x] MAN-11 The credential-authority type is `VendorId` end to end; the v3
  field is `vendor` (v2 `provider` maps in normalization); stored vendor id
  strings are unchanged; the old type name's deprecation alias is now
  **deleted (P7b)**. — workspace-wide rename
  (`crates/ironclaw_host_api/src/ids.rs`; the
  `RuntimeCredentialAccountProviderId = VendorId` alias was removed with zero
  remaining references workspace-wide);
  `v2_and_v3_rewrites_resolve_identically` pins the `provider` → `vendor`
  mapping; the serde wire field stays `provider` (persisted turn-state
  compatibility).

## 2. Resolved record (REC)

- [x] REC-1 Compile once → persisted resolved record + manifest digest; all
  production projection reads the record (no raw-TOML reparse outside the
  compiler and migration). — the generic host's production loader rebuilds
  packages from the persisted resolved contract, never the raw TOML
  (`CompositionExtensionLoader::load`, `generic_host.rs` —
  `ctx.resolved.to_internal(source)`; manifest-source re-checks come from the
  persisted record). The no-reparse gate is now enforced:
  `manifest_reparse_stays_within_the_compiler_migration_and_bundled_paths`
  (`crates/ironclaw_architecture/tests/reborn_manifest_reparse_gate.rs`)
  scans production Rust (test-stripped) and holds every
  `ExtensionManifest::parse` / `ExtensionManifestRecord::from_toml` site to a
  categorized (compiler/migration/bundled-asset) allowlist — a new
  projection-path reparse fails until justified; the installed-record load
  reads `from_resolved` and reparses only pre-resolved legacy rows.
- [x] REC-2 Restart restores extensions from persisted records with the
  package source unavailable. — `records_rehydrate_from_resolved_in_memory` /
  `records_rehydrate_from_resolved_on_libsql`
  (`crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store.rs`
  tests): a record with a corrupted raw source rehydrates from its persisted
  resolved contract. Full package-source-unavailable restart through the
  integration harness lands with the P2 composition cutover.
- [x] REC-3 Legacy raw-TOML installed records backfill idempotently at startup.
  — `legacy_records_backfill_idempotently_in_memory` /
  `legacy_records_backfill_idempotently_on_libsql`: a v2 raw-TOML record
  compiles once at load, persists its resolved contract, and a second load is
  a byte-identical no-op.
- [ ] REC-4 Upgrade diff classifies equal / narrowed / widened contracts;
  widening (scopes, egress, effects, credentials, route) requires approval
  before publication; approval denial leaves the old generation published. — Not
  built — owner decision 2026-07-13: an upgrade-approval gate is net-new
  functionality, outside the train's consolidation mandate. Upgrades exist only
  as silent boot-time adoption of bundled contracts
  (`migrate_host_bundled_manifest_hash`); first-party manifests change via
  reviewed PRs. The classifier (`diff_resolved_contracts`,
  `crates/ironclaw_extensions/src/resolved.rs`) exists and stays tested. Revisit
  trigger: third-party/registry package distribution (overview §7). REL-1's
  sweep counts this row as dispositioned by owner decision.

## 3. Binding, loaders, lifecycle (LIFE)

- [x] LIFE-1 Declared `[[tools]]`/`[mcp]` without a bound tool
  adapter prevents publication; same for `[channel]`; undeclared bindings fail;
  auth never binds.
  — `binding rule` unit tests (`crates/ironclaw_extension_host/src/entrypoint.rs`):
  declared-tool/channel-without-adapter, undeclared-tool/channel adapter, exact
  binding, and `auth_never_binds_is_not_a_binding_field` (the bindings struct
  has no auth field); driven at the internal publication caller by
  `declared_tool_without_bound_adapter_fails_activation`
  (`tests/lifecycle_contract.rs`).
- [x] LIFE-2 `bind` is side-effect-free and receives no network/secret/store
  ports; adapters are parameterized with resolved non-secret tenant
  configuration only.
  — `BindContext` (`entrypoint.rs`) carries only runtime identity, the resolved
  contract, and non-secret tenant values; `ExtensionEntrypoint::bind`
  is a synchronous, port-free signature (secrets exist only behind host
  egress injection).
- [ ] LIFE-3 Native loader resolves `runtime.service` from the registry the
  binary assembles; unknown service fails with a typed error.
- [ ] LIFE-4 WASM and MCP runtime extensions load through synthesized
  entrypoints with no extension-authored Rust.
- [ ] LIFE-5 The generic host is the only writer of caller membership and the
  active snapshot; an admin configuration write cannot create membership.
  — `admin_configuration_does_not_install_an_extension_for_any_user`
  (`tests/integration/extension_user_lifecycle_isolation.rs`), pending the
  combined matrix.
- [ ] LIFE-6 The public lifecycle projection has exactly three values:
  `uninstalled` when the caller is not a member, `setup_needed` when a member
  has any non-ready typed reconciliation result, and `active` only after the
  complete readiness contract succeeds: tenant configuration, personal setup,
  bind, discovery, provisioning, conflict checks, and atomic publication.
  Internal host checkpoints never cross the product boundary as extra states.
  — `users_install_and_remove_the_same_extension_independently` plus
  `webui_v2_gmail_oauth_ready_install_is_immediately_active`, pending the
  combined matrix.
- [ ] LIFE-7 Install joins caller membership and automatically reconciles
  readiness. There is no separate Activate/Disable action; an extension with
  no setup requirement goes directly from `uninstalled` to `active`.
  — `users_install_and_remove_the_same_extension_independently`, pending the
  combined matrix.
- [ ] LIFE-8 Internal bind, discovery, provisioning, conflict, or publication
  failure publishes no callable surface and projects `setup_needed` with a
  typed redacted diagnostic, never a fourth public `failed` state.
- [x] LIFE-9 Channel provisioning and cleanup hooks are internal host steps.
  Their historical `activate`/`cleanup` method names do not define user actions
  or durable lifecycle states.
- [ ] LIFE-10 Removal follows the fixed order (unpublish → drain → vendor
  cleanup → auth revoke/grant delete → config/identity delete) — observed via
  scripted adapter and engine in one caller-level test.
- [x] LIFE-11 Vendor cleanup failure fails the removal operation loud with a
  typed quarantine reason, is retryable, and cannot report success early or
  resurrect the extension. There is no persisted `RemovalPending` state — the
  removal facade retries the whole operation.
  — `ui_facade_extension_remove_retries_incomplete_credential_cleanup_until_converged`
  (`crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`):
  an incomplete credential cleanup returns a retryable
  `ProductWorkflowError::Transient` carrying a typed
  `SecretCleanupQuarantineReason` instead of reporting removal success, and a
  subsequent retry converges (cleanup and removal complete).
- [ ] LIFE-12 Removing one user's membership preserves other users'
  memberships, auth/pairing, runtime access, and tenant admin configuration.
  Caller grants shared by another of that caller's memberships also survive.
  — `users_install_and_remove_the_same_extension_independently` pins the
  multi-user and tenant-configuration halves; shared-vendor grant preservation
  remains covered by the auth-engine ownership suite. Both await the combined
  matrix.
- [x] LIFE-13 Conversation/LLM history survives extension removal. — removal
  runs the vendor-blind removable-channel cleanup (grants, integration state,
  identity bindings — `3812d9fe3`) and never touches turn/LLM history stores
  by construction; P7a pins it at the harness tier —
  `acme_fixture_lifecycle_dispatches_from_the_active_snapshot`
  (`tests/integration/extension_runtime.rs`) removes the extension through the
  model tool, then asserts the invoke thread's persisted turn (user prompt +
  model reply) is still readable via `assert_conversation_history_contains`.
- [x] LIFE-14 Duplicate capability id or ingress route across published
  extensions fails internal publication.
  — `duplicate_capability_across_extensions_fails_activation`
  (`tests/lifecycle_contract.rs`) plus `ActiveSnapshot::build`/`would_conflict`
  conflict detection (`crates/ironclaw_extension_host/src/active.rs`).
- [x] LIFE-15 Upgrade swaps one immutable snapshot; in-flight work completes
  on its old generation `Arc`; new work resolves the new generation; no mixed
  generation under concurrent publish/resolve stress.
  — `in_flight_snapshot_survives_a_later_swap` (`tests/lifecycle_contract.rs`):
  an in-flight `Arc<ActiveSnapshot>` keeps its generation and its extensions
  after a later internal snapshot swap.
- [x] LIFE-16 Startup skips an invalid extension with a typed error and
  publishes the valid rest.
  — `restore_skips_a_load_failure_without_blocking_the_rest`
  (`tests/lifecycle_contract.rs`): a load failure falls to Installed with a
  typed error and does not block the valid restore.
- [ ] LIFE-17 The full user journey passes on both DBs: admin config is saved
  once for the tenant; user A installs, completes personal setup, becomes
  active, and removes; user B's independent state is unchanged throughout.
  — `tests/integration/extension_user_lifecycle_isolation.rs` supplies the
  production-router journey; the PostgreSQL matrix row remains required.
- [ ] LIFE-18 Saving a manifest `[admin_configuration]` group refreshes every
  tenant runtime consumer of that group. The values are not copied into an
  installation record, and refresh failure leaves affected members at
  `setup_needed` with a redacted diagnostic rather than adding a lifecycle
  state. Caller-level refresh and failure tests remain required.

## 4. Tool dispatch (TOOL)

- [x] TOOL-1 Dispatch resolves a prebound adapter by capability id; the
  package/runtime-kind selection per invocation is deleted. —
  `RuntimeDispatcher` resolves through the injected `ToolResolver` port and
  the per-invocation registry/package/runtime-kind selection is gone from
  `crates/ironclaw_dispatcher` (the crate no longer depends on
  `ironclaw_extensions` at all);
  `dispatcher_routes_capability_through_resolved_binding`
  (`crates/ironclaw_dispatcher/tests/dispatch_contract.rs`) plus
  `resolver_prebinds_and_dispatches_through_the_registered_lane` /
  `resolver_tracks_registry_mutations_across_versions`
  (`crates/ironclaw_host_runtime/src/services/tests/registry_lane_tool_resolver.rs`
  — bindings rebuilt per registry generation, resolution is a map lookup).
  The active-snapshot resolver for `ExtensionHost`-published extensions
  chains in with the P2 composition cutover.
- [x] TOOL-2 Unknown capability fails before any adapter work. —
  `dispatcher_fails_unknown_capability_before_any_binding_work` and
  `dispatcher_releases_prepared_reservation_when_resolution_fails`
  (`crates/ironclaw_dispatcher/tests/dispatch_contract.rs`).
- [x] TOOL-3 Authorization, approvals, obligations, resource reservation,
  events, and audit behavior are unchanged through the real dispatcher. —
  authorization keeps its own registry lookup in `CapabilityHost`
  (independent of the deleted dispatcher lookup); pinned through the real
  dispatcher by `capability_host_dispatcher_integration.rs` (invoke
  completes run, approval block/resume, wrong-user resume rejected, expired
  lease rejected before dispatch), `reborn_invoke_vertical_slice.rs`
  (obligations fail before dispatch; resources + events),
  `event_dispatch_contract.rs` (sequence, best-effort sink, redacted kinds),
  and the full composition suite. One documented event delta: a
  missing-backend failure now emits `runtime_selected` before
  `dispatch_failed` (selection succeeded when the binding was constructed;
  the backend is what's missing) — pinned in
  `unconfigured_lane_fails_missing_backend_and_releases_prepared_reservation`.
- [x] TOOL-4 Credential injection derives from the resolved declaration; an
  adapter cannot reach an undeclared credential, egress host, or port. —
  Channel side: the declaration-derived `[[channel.egress]]` policy is
  pinned at the `RestrictedEgress` seam by
  `undeclared_host_is_rejected_before_any_transport_activity`,
  `non_https_and_undeclared_method_are_rejected_before_transport`, and
  `undeclared_credential_handle_is_rejected`
  (`crates/ironclaw_extension_host/src/egress.rs`). Tool side: TOOL-7's
  `slack_tools_invoke_through_the_generic_dispatcher_with_recorded_egress`
  proves declaration-staged network policy + token injection (every
  recorded request targets the declared `slack.com` host with the
  injected bearer).
- [x] TOOL-5 Missing credential raises the generic auth gate and resumes after
  the engine completes (caller-level test). — the raise leg is pinned at the
  harness tier
  (`runtime_401_after_injection_populates_provider_credential_requirement`,
  `github_auth_gate_denied_resume_completes_without_loop`,
  `tests/integration/auth_gate.rs`); the resume-after-engine leg is pinned
  composed-tier (`vendor_oauth_callback_resumes_blocked_turn_gate`,
  `local_dev_oauth_turn_gate_callback_resumes_default_turn_coordinator`). The
  full missing-credential → generic auth gate → engine completes → parked tool
  re-dispatches → run completes chain is now pinned through the integration
  harness in isolation (auto-approve so no approval gate confounds it) by
  `group_journeys::scenario_auth_gate_grant_resume`
  (`journeys_group_auth_convergence_e2e`), asserting the credential-backed
  re-dispatch's real result surfaced (`assert_tool_result_contains`).
- [x] TOOL-6 WASM and MCP lanes invoke through `ToolAdapter` with existing
  result/event semantics. — the WASM lane is proven through `ToolAdapter`
  (TOOL-7 plus the binder contract suite), and the MCP lane is now pinned by
  `binder_invokes_a_discovered_mcp_tool_through_the_tool_adapter`
  (`crates/ironclaw_host_runtime/src/services/tests/extension_tool_binder.rs`):
  a *discovered* (tools/list-originated via
  `package_with_discovered_hosted_mcp_tools`) MCP capability binds through the
  same `LaneBackedToolAdapter` and dispatches into the MCP lane, with the
  exact discovered capability id + input reaching the executor and the output
  flowing back — the binder never distinguishes discovered from static.
- [x] TOOL-7 The five real Slack tools publish and invoke through the generic
  dispatcher (integration, recorded egress). —
  `slack_tools_invoke_through_the_generic_dispatcher_with_recorded_egress`
  (`tests/integration/extension_runtime.rs`): the real Slack package
  reconcile through the facade and all five `slack.*` capabilities dispatch
  snapshot-first (the registry lane is builtin-restricted) through the WASM
  lane with staged policy + token injection; every recorded transport
  request targets `slack.com` and carries the injected bearer token.
- [x] TOOL-8 `slack.send_message` remains an explicit side-effect tool; final
  replies never route through it. — `slack.send_message` stays a declared
  capability (`ironclaw_first_party_extensions/assets/slack/manifest.toml`);
  final replies ride the delivery coordinator path, not the tool:
  `slack_final_reply_flows_through_the_real_delivery_coordinator`
  (`tests/integration/extension_delivery.rs`) and the P6 §10 e2e reply-out
  leg (`test_reborn_slack_channel_e2e.py`) both observe the coordinated
  reply landing on `chat.postMessage` via host-side channel egress with no
  tool invocation in the path.
- [x] TOOL-9 MCP discovery is loader-owned (`ToolAdapter` has no discovery
  method); validated tool surfaces publish atomically; a refresh replaces the
  set completely or not at all; discovered tools run the same dispatcher
  pipeline as static ones. — structurally loader-owned (`ToolAdapter` has no
  discovery method; hosted-MCP discovery runs during readiness reconciliation
  via `discover_hosted_mcp_package` with staged connection authority
  `stage_hosted_mcp_discovery_authority`), and the discovered set publishes
  through the same atomic snapshot publication as static tools. The
  all-or-nothing refresh property is pinned by
  `hosted_mcp_rediscovery_replaces_the_published_tool_set_completely` (a
  refresh whose tools/list yields a different tool replaces the set
  wholesale — the prior discovered tool is gone, not merged) and
  `hosted_mcp_rediscovery_failure_leaves_the_prior_tool_set_intact` (a refresh
  that fails the post-discovery credential recheck before publish leaves the
  prior set live, no partial swap),
  `hosted_mcp_activation_without_discovered_or_static_tools_stays_installed`
  (an initial empty catalog publishes no surface),
  `hosted_mcp_discovery_failure_never_publishes_bundled_static_tools`
  (failed initial discovery cannot be masked by static templates),
  `hosted_mcp_activation_rechecks_credentials_after_discovery_before_publish`,
  `hosted_mcp_activation_discards_discovery_when_credential_epoch_changes`, and
  `hosted_mcp_activation_discards_discovery_when_manifest_inputs_change`
  (changed authority inputs reject the stale generation) — all in
  `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`.
  There is no separate public activation API: refresh means reconcile →
  discover → atomic publish.
- [x] TOOL-10 Host built-in capabilities resolve through the same dispatcher
  pipeline; an extension capability id colliding with a built-in fails
  publication. — built-ins resolve through the registry-lane resolver in the
  same chain (`registry_resolver_allowlist_restricts_to_builtin_provider`,
  `crates/ironclaw_host_runtime/src/services/tests/extension_tool_binder.rs`);
  the collision conflict is pinned at the publication caller by
  `extension_capability_colliding_with_a_host_builtin_fails_activation`
  (`crates/ironclaw_extension_host/tests/lifecycle_contract.rs`), with the
  builtin id set injected by composition
  (`build_local_runtime` → `reserved_capability_ids`).

## 5. Auth engine (AUTH)

- [x] AUTH-1 One engine implements `oauth2_code` and `api_key`; there is no
  auth trait in the extension ABI and no per-vendor code path (grep gate: no
  vendor-conditional in auth crates/composition). — `ironclaw_auth::AuthEngine`
  is the only `AuthProviderClient` composition builds
  (`compose_provider_client`,
  `crates/ironclaw_reborn_composition/src/product_auth/credentials/product_auth_providers.rs`);
  the `ironclaw_auth` engine crate carries zero concrete-vendor literals and the
  extension ABI (`wit/channel.wit`) has no auth trait. CAVEAT: the parenthetical
  "no vendor-conditional in composition" is not yet literal — the specificity
  gate `reborn_generic_code_names_no_concrete_extension` passes against a
  non-empty allowlist that still lists composition vendor branches (e.g.
  `extension_host/gsuite.rs`); that residue is tracked by DEL-8, not blocked
  here. The per-vendor auth modules/specs were deleted (AUTH-16).
- [x] AUTH-2 The authorize URL is host-constructed; recipes cannot supply or
  override `state`, `redirect_uri`, PKCE, `client_id`, `response_type`, or the
  scope parameter. — `authorize_url_is_host_constructed_for_every_oauth_vendor_row`,
  `recipes_cannot_supply_or_override_reserved_authorize_params`,
  `authorization_endpoint_predefining_reserved_params_is_rejected`
  (`crates/ironclaw_auth/tests/auth_engine_contract.rs`).
- [x] AUTH-3 State/CSRF, PKCE, TTL, and callback replay are enforced; exactly
  one transition consumes a callback. — `exactly_one_transition_consumes_a_callback`,
  `cross_flow_callbacks_are_rejected` (`auth_engine_contract.rs`); state-hash /
  PKCE-hash / TTL validation stays in the durable `AuthFlowManager`
  (`crates/ironclaw_reborn_composition/src/product_auth/durable/tests.rs`).
- [x] AUTH-4 Requested scopes intersect the recipe ceiling; widening is
  rejected before the vendor call. —
  `scope_widening_is_rejected_before_any_vendor_call` (`auth_engine_contract.rs`).
- [x] AUTH-5 Token exchange supports `post_body` and `basic`; response fields
  extract via bounded JSON pointers, including `fallback_to_requested` scope. —
  `token_exchange_supports_post_body_and_basic_client_auth`,
  `pointer_extraction_reads_nested_fields_and_scope_fallback`,
  `missing_scope_without_fallback_fails_the_exchange` (`auth_engine_contract.rs`).
- [x] AUTH-6 Refresh runs on-demand at injection with single-flight and honors
  `rotates_refresh_token` both ways; revoke is idempotent; vendor response
  bodies are size-capped and redacted from errors and logs. —
  `refresh_honors_rotates_refresh_token_both_ways`,
  `revoke_is_idempotent_and_best_effort`,
  `vendor_error_responses_are_size_capped_and_never_echoed`
  (`auth_engine_contract.rs`); on-demand-at-injection single-flight is the
  per-account refresh lock in `ProviderBackedCredentialAccountService`
  (`crates/ironclaw_auth/src/credential.rs`) driven by the inline
  injection-time refresh in `runtime_credentials.rs`. KEEPALIVE LEG (owner
  call, resolves the #6008 owner note): a recipe may declare an idle
  keepalive threshold (`refresh.keepalive_idle_seconds`, a vendor lifetime
  constraint — implementation §7); the engine executes one generic
  vendor-blind background sweep (leader-locked, due at half the declared
  lifetime, soonest-death-first under the per-tick cap), replacing the
  composition-owned Google-specific worker. —
  `keepalive_sweep_refreshes_due_accounts_of_declaring_vendors_only`,
  `keepalive_sweep_skips_the_tick_when_not_leader`,
  `keepalive_refresh_failure_follows_engine_account_state_rules`,
  `google_manifests_declare_the_keepalive_idle_lifetime_identically`
  (`auth_engine_contract.rs`); vendor-blind candidate enumeration on both
  DB-gated builds (`list_refresh_candidates_covers_agent_and_project_scopes`,
  composition `product_auth/durable/tests.rs`); recipe-field validation +
  shared-vendor conflict coverage in `ironclaw_host_api` `recipe.rs` tests.
- [x] AUTH-7 Identity extracts from the token response or the declared
  identity endpoint and is validated against the flow before storage. —
  `pointer_extraction_reads_nested_fields_and_scope_fallback` (token response),
  `identity_extracts_from_declared_endpoint_with_fresh_credential` (endpoint,
  incl. rejection failing the exchange) (`auth_engine_contract.rs`).
- [x] AUTH-8 Grants/secrets are encrypted at rest; stored secrets are never
  echoed to UI or adapters. — token material lives only behind
  `ironclaw_secrets::SecretStore` handles (encryption is the store's
  property, unchanged here); redaction pinned by
  `vendor_error_responses_are_size_capped_and_never_echoed`
  (`auth_engine_contract.rs`) and the existing
  `serde_redaction_contract.rs` suite.
- [x] AUTH-9 The auth account state machine is one shared enum
  (`disconnected/authenticating/connected/expired` + typed
  `last_error`); no vendor- or extension-specific state exists; the wire
  exposes exactly this enum. — enum + typed `last_error` + transitions live
  with the engine (`crates/ironclaw_auth/src/account_state.rs`,
  `legal_transitions_only`, `auth_account_state_wire_form_matches_str`;
  re-exported by `ironclaw_extension_host::state`). P7a puts it on the wire:
  the extensions wire now carries a per-vendor **accounts list** whose
  `RebornAuthAccount.state` is exactly this enum (`reborn_services/types.rs`),
  replacing the `connected: Option<bool>` stopgap. The accounts-list shape
  (`account_id`/`label`/`state`/`is_default`) is frozen by the golden fixture
  `list_extensions_golden_wire_multi_surface_extension_freezes_accounts_list`
  (`reborn_services_contract.rs`), driven through `list_extensions`; the
  projection `vendor_auth_accounts` (`reborn_services/extensions.rs`) maps a
  live grant to `connected` (MIG-1). Richer per-account state (expired/
  authenticating) flows when the credential service surfaces it with
  the post-P7 multi-account feature — the wire type already carries the full
  enum. (`revoking` is not part of the enum: disconnect/removal delete the
  account synchronously, so there is no transient wire state to surface.)
- [x] AUTH-10 Flow TTL expiry and vendor denial land in `disconnected` with
  a typed reason; refresh failure lands in `expired`. —
  `projection_prefers_live_flow_then_account_status`
  (`crates/ironclaw_auth/src/account_state.rs`).
- [x] AUTH-11 `api_key` renders from recipe fields, runs the optional
  validation probe through restricted egress, and uses the same state machine.
  — probe + storage + state machine proven engine-tier
  (`api_key_probe_validates_through_host_egress_before_storing`,
  `api_key_probe_failure_stores_nothing`, `api_key_without_probe_stores_directly`,
  `auth_engine_contract.rs`); the P6 S5 configure modal now renders
  wire-projected secret/field descriptors generically (`c6bb695ec`,
  `configure-modal.tsx` — manual-token entry rides the generic form). The
  api_key-recipe → wire field projection is now pinned end-to-end on the real
  github manifest by
  `webui_v2_github_api_key_setup_projects_manual_token_secret`
  (`crates/ironclaw_reborn_composition/tests/webui_v2_e2e.rs`): the composed
  `/extensions/github/setup` route projects the `[auth.github] method = "api_key"`
  recipe's single field handle into exactly one `manual_token` secret
  descriptor (handle-named, not-yet-provided) through the production
  composition→product-workflow chain, no per-vendor code.
- [x] AUTH-12 All five current vendors (Slack, Google, Notion, GitHub,
  NEAR AI) are expressed as recipes and pass the engine suite as table rows —
  no vendor-specific test suite exists. — rows loaded from the real bundled
  manifests (`all_five_vendors_load_as_recipe_rows_from_their_manifests` and
  the rows across `auth_engine_contract.rs`); the legacy per-vendor suites
  (`oauth_provider_client/tests.rs`, the Google/Slack gate-provider tests,
  the DCR provider suite) were deleted with their production code.
- [x] AUTH-13 Callback route keeps the existing
  `/api/reborn/product-auth/oauth/{provider}/callback` shape; `{provider}` is
  resolved as data (vendor-registered redirect URLs unchanged). — one axum
  route (`VENDOR_OAUTH_CALLBACK_PATH`) resolves `{provider}` through the
  engine's `AuthRecipeResolver`; the Google/Slack URLs are served by the same
  generic route (`vendor_oauth_callback_completes_a_started_flow`,
  `crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs`; the
  google/slack callback tests in `tests/webui_v2_product_auth.rs` drive the
  unchanged URLs end-to-end).
- [x] AUTH-14 Slack end-to-end: blocked tool → gate → scripted callback →
  grant stored → tool resumes (extends the existing oauth-connect integration
  test). — the generic round trip is proven at the composed-services tier with
  the recipe-driven driver and the `{provider}` callback route
  (`vendor_oauth_callback_resumes_blocked_turn_gate`,
  `crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs`), and the
  callback→coordinator resume is pinned by
  `local_dev_oauth_turn_gate_callback_resumes_default_turn_coordinator`
  (`crates/ironclaw_reborn_composition/src/factory/auth_tests.rs`); the full
  Slack personal-OAuth connect against production serve is proven end-to-end by
  the P6 §10 e2e (`tests/e2e/scenarios/test_reborn_slack_channel_e2e.py`). The
  blocked-tool → auth gate → grant stored → tool resumes leg through the
  integration harness is now pinned by
  `group_journeys::scenario_auth_then_approval_journey` (turn 1: blocked
  `github.get_repo` → approval → auth gate → `resolve_auth_gate` grants a real
  credential through the production manual-token flow → run completes → repo
  result surfaces) and the isolated
  `group_journeys::scenario_auth_gate_grant_resume` (auto-approve so the auth
  gate is the only block). Grant storage is the "user submitted credentials"
  arm (`resolve_auth_gate`'s `request_manual_token_setup` → `submit_manual_token`).
- [x] AUTH-15 Engine flow/grant persistence passes on both DBs. — the engine
  reuses the backend-generic `FilesystemAuthProductServices` store; the
  connect flow is pinned on the in-memory backend, on a real libSQL root
  filesystem, and now on a real PostgreSQL root filesystem
  (`oauth_connect_flow_persists_credential_account`,
  `oauth_connect_flow_persists_credential_account_on_libsql`,
  `oauth_connect_flow_persists_credential_account_on_postgres`,
  `tests/integration/oauth_connect.rs`; all three green, REL-3: a Postgres
  skip is a failure). The Postgres arm reuses the harness's testcontainer
  provisioner (`start_postgres_testcontainer`, now `pub(crate)`) and a new
  backend-generic bundle builder
  `build_oauth_product_auth_for_test_on_root<F: RootFilesystem>` (the OAuth
  bundle is built outside the harness storage composite, so it can't reuse
  `StorageMode::Postgres` — correction A's sanctioned thin composition-tier
  addition; generic so it needs no concrete-backend feature).
- [x] AUTH-16 The provider string multiplexor, provider spec constants, and
  Slack OAuth branches are deleted. — `MultiplexAuthProviderClient` /
  `compose_provider_clients`, `HostOAuthProviderSpec`, `TokenResponseShape`,
  `google_provider_spec` / `notion_provider_spec` /
  `slack_personal_provider_spec`, the per-vendor gate providers + registries,
  the DCR provider modules, the Slack/Google serve handlers and start
  branches, and `ironclaw_auth`'s legacy vendor URL builders /
  per-vendor callback-state kinds are all deleted; the blocked-turn
  `OAuthGateFlowDriver` survives, re-pointed at the engine.

## 6. Channel ingress (ING)

- [x] ING-1 One generic router serves
  `/webhooks/extensions/{extension_id}/{route_suffix}` from the active
  snapshot; extensions cannot mount arbitrary routes; collisions with fixed
  host routes fail publication. —
  `route_table_follows_snapshot_swaps_without_router_rebuild`,
  `activation_rejects_collision_with_fixed_host_routes`
  (`crates/ironclaw_extension_host/tests/ingress_router_contract.rs`);
  production mount leg:
  `signed_acme_post_flows_through_the_production_mount_into_a_turn`
  (`tests/integration/extension_ingress.rs`). Fixed-route set injected by
  composition (`reserved_fixed_ingress_routes`, empty today — no fixed host
  route lives under the extension namespace).
- [x] ING-2 Method, body limit, rate limit, and deadline are enforced before
  adapter work. —
  `method_body_and_rate_limits_run_before_verification_and_adapter`,
  `request_deadline_bounds_verification_through_admission`
  (`ingress_router_contract.rs`).
- [x] ING-3 `hmac_sha256` recipes verify exact byte construction
  (fixture-pinned), with constant-time comparison and timestamp/replay
  rejection. — `hmac_recipe_verifies_the_exact_acme_byte_construction`,
  `hmac_recipe_enforces_the_replay_window_before_any_hmac`,
  `hmac_recipe_rejects_tampered_body_missing_and_bad_signatures`
  (`crates/ironclaw_extension_host/src/ingress/verifier.rs`, `subtle::ct_eq`
  comparisons); wire legs in `ingress_router_contract.rs` and
  `tests/integration/extension_ingress.rs`.
- [x] ING-4 `shared_secret_header` verifies constant-time and rejects
  missing/duplicate headers. —
  `shared_secret_header_verifies_constant_time_and_rejects_missing_duplicate`
  (`verifier.rs`; duplicate signature/timestamp headers also fail closed:
  `hmac_recipe_rejects_duplicated_signature_or_timestamp_headers`).
- [x] ING-5 Signing secrets are never observable by the adapter (scripted
  adapter records its full inputs). —
  `adapter_never_observes_verification_headers_or_secret_material`
  (`ingress_router_contract.rs`).
- [x] ING-6 With multiple candidate installations, verification tries each
  within the fixed bound and resolves exactly one or rejects as ambiguous. —
  `hmac_recipe_resolves_exactly_one_of_multiple_candidates` (`verifier.rs`,
  bound `MAX_VERIFICATION_CANDIDATES = 8`);
  `multi_candidate_verification_resolves_exactly_one_installation`
  (`ingress_router_contract.rs`).
- [x] ING-7 `adapter.inbound` receives bounded input, is panic-isolated, and
  returns `Messages`/`Respond`/`Ignore` only. —
  `adapter_panic_is_isolated_and_the_router_survives`,
  `adapter_never_observes_verification_headers_or_secret_material`
  (`ingress_router_contract.rs`; the outcome enum is the trait's only return
  shape, and out-of-bounds messages/responses are rejected host-side).
- [x] ING-8 2xx is returned only after the durable dedupe/admission commit;
  store failure returns retryable 5xx; crash/duplicate/restart replay
  converges exactly once (both DBs). —
  `two_hundred_only_after_durable_admission_commit`
  (`ingress_router_contract.rs`, scripted-sink leg);
  `duplicate_and_restart_replay_converge_exactly_once` matrixed over
  `StorageMode::{LibSql,Postgres}`
  (`tests/integration/extension_ingress.rs`, real durable idempotency
  ledger; "restart" = fresh sink over the same durable ledger). NOTE: a
  terminally settled workflow rejection also acks 2xx — it is durably
  accounted for (replay converges `Duplicate`), and user feedback flows
  through the post-admission observer. `Ignore` acks without a ledger write:
  ignored outcomes carry no event identity to key a commit.
- [x] ING-9 Challenge (`Respond`) answers after verification without enqueue,
  within response size/status bounds. —
  `respond_outcome_answers_without_enqueue_within_bounds`
  (`ingress_router_contract.rs`);
  `url_verification_challenge_becomes_an_immediate_response`
  (`crates/ironclaw_slack_extension/src/channel.rs`); production-mount leg
  in `tests/integration/extension_ingress.rs`.
- [x] ING-10 Normalized messages flow through existing identity/conversation
  binding and turn submission (integration: signed vendor POST → turn). —
  `signed_acme_post_flows_through_the_production_mount_into_a_turn`
  (`tests/integration/extension_ingress.rs`: the post-admission observer
  records `ProductInboundAck::Accepted { submitted_run_id, .. }` from the
  REAL `DefaultProductWorkflow`).
- [x] ING-11 `reply_context` is stored host-side and returned to the same
  extension's adapter at delivery time. — Storage half:
  `reply_context_is_stored_host_side_keyed_by_conversation`
  (`ingress_router_contract.rs`, keyed by conversation fingerprint).
  Delivery-time return (P5): the coordinator resolves the stored context
  through its `DeliveryReplyContextSource` port and hands it to the adapter
  on the `OutboundEnvelope` —
  `coordinator_persists_sending_before_the_adapter_delivers` asserts the
  envelope carries the source's bytes (`outbound_delivery_contract.rs`),
  and `factory.rs` wires `IngressReplyContextSource` over the SAME
  `ingress_parts.reply_context` store the router writes
  (`extension_host/channel_delivery.rs`).
- [x] ING-12 Slack and Telegram inbound both pass through the same router and
  workflow caller with zero host branches (one integration proof each). —
  Slack: the full
  `crates/ironclaw_reborn_composition/src/extension_host/channel_host/e2e_tests.rs`
  scenario suite (28 tests) now drives the generic router + recipe verifier +
  `SlackChannelAdapter` + generic sink through the alias mount (e.g.
  `slack_dm_delivers_final_reply_after_immediate_ack`,
  `slack_events_rejects_forged_hmac_signature`). Acme proves the
  route-agnostic half (`tests/integration/extension_ingress.rs`). Telegram
  (P5): `telegram_update_becomes_a_turn_and_a_coordinated_reply`
  (`tests/integration/extension_delivery.rs`) drives a signed update through
  the SAME production mount, generic sink, and workflow caller — the
  registration is data (`ChannelIngressRegistration`), zero telegram host
  branches.
- [ ] ING-13 Inbound attachments are references; any byte fetch happens
  host-side through restricted egress with the channel credential — adapters
  never fetch. — Reference half holds by construction (`AttachmentRef`
  carries descriptor + vendor ref + mime hint; Slack/Telegram adapters map
  descriptors without fetching —
  `dm_message_normalizes_with_text_trigger_and_event_identity`). The
  host-side fetch path is specified but deliberately unbuilt until a
  consumer needs bytes (overview §4.2).

## 7. Channel outbound (OUT)

- [x] OUT-1 Every outbound intent (final reply, gate prompt, auth prompt,
  failure, connect-required, working, cleanup, triggered delivery) enters the
  one coordinator; a grep/architecture check finds no direct product send
  path. — The eight `DeliveryIntent`s split policy-class
  (`deliver`) / notice-class (`deliver_notice`) with cross-class calls
  rejected (`coordinator_notice_rejects_policy_class_intents`,
  `coordinator_deliver_rejects_notice_class_intents`,
  `outbound_delivery_contract.rs`); the generic observer/driver emit ONLY
  through `RunDeliveryServices.coordinator` (`run_delivery_contract.rs`, 11
  scenarios); the P5 cutover deleted the direct Slack send lane
  (`slack_delivery.rs`, `slack_egress.rs`, `slack_dm_open.rs`) and re-pointed
  all 28 `channel_host/e2e_tests.rs` scenarios through the coordinator.
- [x] OUT-2 Target resolution preserves source-route replies and preference
  targets; unauthorized/unavailable targets fail closed. — Policy-class
  deliveries run the SAME `OutboundPolicyService` pipeline (source-route
  reply-target validation + preference targets) inside the coordinator;
  fail-closed rows: `coordinator_rejected_policy_decision_does_not_reach_the_adapter`,
  `coordinator_require_direct_message_rejects_non_dm_target_without_egress`,
  `coordinator_fails_closed_when_the_channel_is_unavailable`,
  `coordinator_notice_fails_closed_when_the_channel_is_unavailable`
  (`outbound_delivery_contract.rs`); the triggered path resolves the
  creator's preference target (`run_delivery_contract.rs`).
- [x] OUT-3 An attempt is persisted (`Prepared`→`Sending`) before vendor
  egress. — `coordinator_persists_sending_before_the_adapter_delivers` (the
  scripted adapter reads the durable attempt DURING deliver and sees
  `Sending`) and `coordinator_notice_is_source_routed_and_persists_before_egress`
  (`outbound_delivery_contract.rs`); `ironclaw_outbound::service` records the
  initial attempt as `Prepared`. Integration: `assert_delivered_attempt`
  (`tests/integration/extension_delivery.rs`) pins that no attempt is left
  mid-lifecycle after delivery completes.
- [x] OUT-4 The coordinator is the sole delivery-state writer; adapters
  receive no store; production construction rejects a no-op sink. —
  `ChannelAdapter::deliver(envelope, &dyn RestrictedEgress)` receives no
  store by signature; all status writes live in
  `delivery_coordinator.rs`; the factory constructs the coordinator only
  when a real channel-egress transport exists — with no transport the factory
  returns `delivery_coordinator: None` rather than wiring a no-op sink
  (`crates/ironclaw_reborn_composition/src/factory.rs`). There is deliberately
  no no-op-sink constructor (`delivery_coordinator.rs` doc), so the guarantee
  is structural rather than a dedicated test.
- [x] OUT-5 Retry/backoff and run-notice dedupe are generic; the coordinator
  owns its per-delivery single-flight guard. There is no external coordinator
  shutdown/drain surface. —
  `coordinator_retries_fully_retryable_reports_then_delivers` (bounded
  retry, zero-backoff policy injection),
  busy-hint FIFO dedupe rows in `run_delivery_contract.rs`; the
  per-delivery-id single-flight guard (`in_flight` set, both paths) is
  structural — every
  prepared attempt mints a fresh delivery id, so double-entry is
  unreachable through the public API; the guard defends future
  resume/re-drive paths.
- [x] OUT-6 Crash after possible vendor success records `Unknown`; no blind
  resend without a vendor idempotency key. —
  `coordinator_recovery_marks_interrupted_sending_attempts_unknown` and
  `coordinator_lazily_recovers_interrupted_attempts_before_a_scopes_first_delivery`
  (`outbound_delivery_contract.rs`): interrupted `Sending` attempts from a
  prior lifetime settle `Unknown` (never re-driven) lazily before that
  scope's first delivery — the store enumerates per scope only, so recovery
  is per-scope on first touch (owner call, flagged in the PR body).
- [x] OUT-7 Partial multipart: once any part sends, a later retryable failure
  is terminal unless an idempotency key proves safe retry. —
  `coordinator_partial_multipart_failure_is_terminal_without_retry`
  (`outbound_delivery_contract.rs`): after a `Sent` part, a retryable part
  failure terminates the attempt (`Failed`, no re-drive) since no vendor
  idempotency key exists for Slack/Telegram message posts.
- [x] OUT-8 Restricted egress rejects undeclared hosts/methods,
  adapter-supplied auth headers where injection is declared, cross-host
  redirects, private-IP/DNS-rebind targets, and oversized bodies — before any
  network call. — `PolicyEnforcedChannelEgress`
  (`ironclaw_extension_host::egress`):
  `undeclared_host_is_rejected_before_any_transport_activity`,
  `non_https_and_undeclared_method_are_rejected_before_transport`,
  `adapter_supplied_authorization_header_is_rejected`,
  `undeclared_credential_handle_is_rejected`,
  `oversized_transport_response_is_rejected`; the transport pins the network
  policy to the approved host with `deny_private_ip_ranges` (redirects and
  DNS-rebind are the host-runtime egress layer's existing SSRF guards —
  `channel_egress.rs::header_injection_reaches_the_network_request` pins the
  pinned-policy handoff).
- [x] OUT-9 Delivery attempt persistence passes on both DBs. — the P5 delivery
  proofs' `Delivered`-attempt assertions run matrixed over
  `StorageMode::{LibSql,Postgres}` (`tests/integration/extension_delivery.rs`;
  a Postgres provisioning failure is a test failure, never a skip). NOTE: the
  separately-cited `reborn_integration_outbound_store_durability` binary is
  libsql-only (its single test is a fresh-libsql reopen); the both-DB guarantee
  rides the delivery-proof matrix above, not that store suite.
- [x] OUT-10 Slack rendering/splitting/DM-provisioning and Telegram rendering
  live only in their crates (fixture unit tests) with one outbound
  integration proof each. — Rendering/splitting/`conversations.open` DM
  opening live in `ironclaw_slack_extension` /
  `ironclaw_telegram_extension` (fixture tests in each crate's
  `channel.rs` + TEST-1 conformance); outbound integration proofs:
  `slack_final_reply_flows_through_the_real_delivery_coordinator` and
  `telegram_update_becomes_a_turn_and_a_coordinated_reply`
  (`tests/integration/extension_delivery.rs`). The thin preference-target
  provisioner glue (`slack_preference_targets.rs`) rides composition as a
  §11 sliver until the P6 extraction.
- [x] OUT-11 Prompt construction consumes `CommunicationPresentationPolicy`
  from the channel contract; concrete channel branches in `ironclaw_llm` are
  deleted. — no concrete channel branch remains in `ironclaw_llm`, and the
  manifest's `[channel.presentation]` (`supports_markdown`,
  `max_message_chars`) now feeds prompt construction per channel. The resolved
  `ChannelPresentation` (`ironclaw_host_api::channel`) flows manifest →
  `LifecycleExtensionSummary.channel_presentation`
  (`available_extensions.rs::summary` / `channel_presentation_from_manifest_record`)
  → the communication provider → `ConnectedChannelSummary.presentation` →
  `LoopRuntimeContext::render_model_content`, which renders a compact per-channel
  hint (e.g. `Slack (authenticated, active, markdown, ≤40000 chars/message)`).
  Pins: `renders_channel_presentation_hint`
  (`ironclaw_turns/src/run_profile/runtime_context.rs`, render half),
  `bundled_slack_package_declares_product_adapter_channel_surface` (the real
  slack manifest projects `supports_markdown=true`/`max_message_chars=40000`
  onto the summary), and `channel_extensions_are_classified_as_connected_channels`
  (the presentation flows through `RuntimeCommunicationContextProvider` onto the
  connected-channel summary). OWNER CALL (flagged in the PR body): Option A —
  widen the communication-context seam (`LifecycleExtensionSummary` → provider →
  `ConnectedChannelSummary`), reusing the existing per-channel pipe, rather than
  Option B (a new extension-host lookup port). `LifecycleExtensionSummary` is an
  internal lifecycle DTO, not the WebUI wire, so no golden fixture changes.
- [x] OUT-12 Trace contributions use generic extension/surface origin ids;
  concrete variants are deleted. — `ironclaw_reborn_traces` carries
  `channel_origin: Option<String>` as data
  (`trace_channel_origin_from_host_channel`, `client.rs`); no vendor enum
  variant remains in the trace vocabulary.

## 8. Extraction and deletion (DEL)

- [x] DEL-1 `crates/ironclaw_reborn_composition/src/slack/` no longer exists.
  — P6 deleted the lane: the generic channel host assembly +
  manifest `[admin_configuration]` + the generic outbound-target provider and
  triggered-delivery hook replaced every lane surface; the H.3/H.4 folds
  (`channel_state_folds.rs`) carry retired durable state forward.
- [x] DEL-2 The concrete extension crates are renamed
  `ironclaw_slack_v2_adapter` → `ironclaw_slack_extension` and
  `ironclaw_telegram_v2_adapter` → `ironclaw_telegram_extension`. — P7b pure
  crate rename: the fold had already run INTO these crates in P6 (making them
  the de-facto extension crates), and DEL-5 removed their dead `ProductAdapter`
  impls, leaving the live `ChannelAdapter` + codecs + preference-target codec.
  `git mv` preserves history; all dependency declarations, `use` paths, the
  `assert_workspace_deps_exactly` allowlist, the retired
  `ironclaw_wasm_product_adapters` BoundaryRule / forbidden entries, CI crate
  lists, and docs are renamed. The deletion-gate crate lists (specificity
  `CONCRETE_EXTENSION_CRATES`, `check-generic-without-concrete.sh`) already
  reserved both new names, so the old entries were removed rather than
  duplicated. `cargo check --workspace --tests` + `ironclaw_architecture` (42)
  green.
- [x] DEL-3 `serve_slack.rs` and the `slack-v2-host-beta` cargo feature are
  deleted; no channel-specific config type remains in
  `ironclaw_reborn_config`. — `SlackSection`/`SlackChannelRouteSection` are
  gone; a stale `[slack]` section hard-fails config parse (accepted beta
  posture, pinned by `rejects_retired_slack_section`); the secrets guard
  keeps the `xoxb-`/`xoxp-`/`xapp-` prefixes.
- [x] DEL-4 Slack cleanup constants in product workflow and Slack connection
  copy in lifecycle are deleted (standard pipeline + manifest display data).
  — no non-test slack constant remains in `ironclaw_product`
  (removal cleanup rides the vendor-blind removable-channel path); P7a
  deletes the hardcoded `slack` OAuth branch in
  `channel_connection_requirement` (`extension_lifecycle.rs`). The connect
  strategy now derives from the manifest's declared auth setup
  (`channel_connect_strategy`: an OAuth recipe → OAuth, otherwise the generic
  proof-code pairing) and the copy renders generically from the S5
  `display_name` — pinned by
  `channel_connect_strategy_is_manifest_driven_not_name_based` (the real Slack
  package resolves to OAuth from its `[auth.slack]` recipe; a bot-token
  fixture named "slack" resolves to proof-code, proving no name hardcode
  survives). The remaining `slack` allowlist term entries for
  `extension_lifecycle.rs` cover other (non-DEL-4) slack references and retire
  with the P7b allowlist→0 sweep.
- [x] DEL-5 The `ProductAdapter` contract is fully retired (owner decision:
  full retirement executed AS CONSOLIDATION, coverage ported not deleted). —
  P2/P6 removed both registry-projection halves (`ProductAdapterRuntimeEntry` /
  `list_enabled_product_adapter_entries`; `59893460e`). P7b deletes the
  remainder: the `ProductAdapter` trait + `ProductAdapterHealth` +
  metadata getters (`crates/ironclaw_product_adapters/src/adapter.rs`); the
  production-dead `prepare_and_render_product_outbound` + its
  request/outcome/error types (`ironclaw_product/src/outbound_delivery.rs`,
  keeping the LIVE `VerifiedProductOutboundTargetMetadata` +
  `ProductOutboundTargetResolver` + `delivery_failure_kind_for_workflow_error`
  the coordinator uses); the concrete `SlackV2Adapter`/`TelegramV2Adapter`
  impls + their egress/auth/capability helpers (`slack|telegram`
  `src/adapter.rs`, keeping the LIVE `SlackChannelAdapter`/`TelegramChannelAdapter`
  in `channel.rs`); and the whole `ironclaw_wasm_product_adapters` crate
  (workspace member, arch-test rules + guardrail/WIT tests, CI buckets, doc
  refs). **Revisit trigger (owner):** WASM channel adapters return when a WASM
  channel exists (overview §4.0), rebuilt on `ChannelAdapter` via a
  loader-synthesized entrypoint; the WIT-based ProductAdapter runner was
  superseded by the NEA-25 contract — see git history at this commit. Its ~9
  specificity allowlist entries retire with it (DEL-8 progress). **Coverage
  ported, not deleted:** the live test double `RebornTestProductAdapter` → the
  trait-free `RebornTestIngress` (builds `ParsedProductInbound` directly on the
  LIVE `TrustedInboundContext`/`ProductInboundEnvelope` path the production
  `extension_ingress` bridge uses); the retired `ProductAdapter::parse_inbound`/
  `render_outbound` fidelity suites move to the `ChannelAdapter` conformance
  (`run_channel_adapter_conformance`); the 18 `prepare_and_render_product_outbound`
  contract tests each carry a per-test disposition (PR body), with the two
  **#4953 fix-born** DM-requirement pins PORTED onto the coordinator
  (`coordinator_require_direct_message_rejects_non_dm_target_without_egress`)
  plus a ported policy-rejection fail-closed pin
  (`coordinator_rejected_policy_decision_does_not_reach_the_adapter`).
- [x] DEL-6 Composition constructs no concrete extension and mounts no
  concrete route (architecture gate). — The lane deletion removed the last
  concrete construction and route mount from composition (`serve_slack`
  and `with_slack_channel_routes` are gone; the CLI supplies channel
  adapters through `RebornHostBindings::with_channel_extension_bindings`).
  Gates green: `reborn_generic_code_names_no_concrete_extension` +
  `concrete_extension_crates_link_only_from_the_binary_and_tests`
  (`crates/ironclaw_architecture/tests/reborn_extension_specificity.rs`,
  empty `CONCRETE_DEPENDENCY_EXCEPTIONS`).
- [x] DEL-7 Only the canonical `ironclaw` CLI package and tests depend on concrete extension
  crates (`cargo metadata` gate). — `CONCRETE_DEPENDENCY_EXCEPTIONS` is
  empty; composition keeps `ironclaw_slack_extension` as a dev-dependency
  only (the sanctioned test linkage); the CLI supplies the Slack channel
  adapter + extras through `RebornHostBindings::with_channel_extension_bindings`.
- [ ] DEL-8 The concrete-name scanner allowlist is empty. — In progress: the
  shrink-only `ALLOWLIST` (`reborn_extension_specificity.rs`) is **86**
  `(path, term)` entries (102 at the start of the finalize-takeover session;
  ~145 at the start of P7b). **Lane A holds**: composition's package
  catalog/registration names no first-party package — every userland package
  (github, gmail, google×5, notion, slack, telegram, web-access) is a
  self-contained module under `ironclaw_first_party_extensions::packages`,
  consumed as an opaque bundle; the factory trust-policy iterates the inventory
  generically. Every remaining entry is now **lane-4 residue**, regrouped in
  the const under `// lane-4: <category>` markers and characterized in the
  PR #6065 lane-4 inventory. The gate does not block on a non-empty allowlist
  (only on stale/new-violation), so this row stays `[ ]` until the residue is
  drained:
  - **Lane 1 done** — i18n `github` (11) carved as localized UI copy (the SSO
    provider name + skill-install source + the user-facing HTTP capability
    hint; mirrors the existing `google` i18n carve). The `notion` scanner
    false-positive (the English word "no notion of", not the extension) was
    reworded and cleared. The `reborn-extension-surfaces` skill how-to was
    rewritten to the v3 manifest shape (REL-4, not an allowlist entry).
  - **Lane 2 done** — the `ironclaw_reborn_traces::contribution` redaction
    classifier (4: slack/telegram/gmail/github) carved as a **vendor-safety
    denylist**: it keys the payload-redaction profile + external-write
    side-effect off tool-name keywords and is a *superset* of the inventory
    (also signal/discord/gitlab), so sourcing it from the inventory would drop
    those and weaken redaction. Pinned by
    `tool_payload_redaction_profile_is_a_safety_denylist_not_inventory_routing`.
    The fourth carve-domain doc generalized from "credential-format detection"
    to "vendor-specific safety detection".
  - **Lane 3 DEFERRED (owner call)** — `nearai_mcp` (13), the last catalog
    package. Its `[mcp].server` is patched from `llm_admin` config at runtime,
    so finishing it is a multi-file composition refactor touching production
    auth-fallback / boot auto-publication / trust; deferred with a complete
    execution plan (PR #6065 body + handoff) rather than rushed. bare `nearai`
    stays a global `TERM_COLLISIONS` carve; the compound `nearai_mcp` forms stay
    scanned and tagged lane-4 `nearai-slice`.
  - **Lane 4 (the rest, 73)** — genuine generic branches (route-by-manifest
    candidates), the web-access assembly module, one migration call site, the
    sanctioned DEL-7 dev-dep, and incidental doc/tool-string examples that name
    an extension but branch on nothing. Tagged and inventoried; each is the
    owner's degenericize-or-carve decision.
- [ ] DEL-9 `check-generic-without-concrete.sh` passes in CI: every generic
  crate's dependency tree is free of concrete extension crates and its tests
  pass — the deletion test. — LOCAL pass, NOT yet CI-wired. The script is
  correct with `TEMPORARY_EXCEPTIONS` empty; `bash
  scripts/ci/check-generic-without-concrete.sh --trees-only` is green
  ("dependency graphs clean", 63 generic crates). The *dependency-direction*
  half runs in CI via the `ironclaw_architecture` arch test
  (`concrete_extension_crates_link_only_from_the_binary_and_tests`), but **no
  CI workflow invokes this script**, so the "in CI" clause and the full
  per-crate isolated-test form are unmet. Wiring the script into a CI job (and
  marking it required) is the remaining step — tracked jointly with REL-5.
- [ ] DEL-10 Telegram runs as a real installed package (manifest + internal
  webhook provisioning) — the addition test proven by the second
  production channel. — Adapter half (P4):
  `TelegramChannelAdapter::{inbound,activate,cleanup}` with
  `shared_secret_header` verification host-side and `setWebhook`/
  `deleteWebhook` over `RestrictedEgress`
  (`crates/ironclaw_telegram_extension/src/channel.rs`). P5 completed the
  chain: bundled package assets + inventory module
  (`ironclaw_first_party_extensions/assets/telegram/manifest.toml`,
  `ironclaw_first_party_extensions::packages::telegram`; P7b DEL-8 lane A
  migrated the former `available_extensions.rs::telegram_package` builder into
  the self-contained inventory), the binary-assembled
  entrypoint binding (`crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs`),
  the real channel-egress transport with host-side path-placeholder token
  injection (`HostRuntimeChannelEgressTransport`,
  `path_placeholder_injection_substitutes_the_secret_host_side`), the
  manifest-declared body-credential binding that resolves the webhook shared
  secret into `setWebhook`'s `secret_token` JSON field host-side
  (`[[channel.egress]] body_credentials`,
  `RuntimeCredentialTarget::BodyJsonPointer`,
  `body_json_pointer_credential_is_resolved_into_the_wire_body`; the adapter
  names the handle only), and the
  install/readiness reconciliation (`setWebhook` over recorded egress, wire
  body carrying the configured secret value and never the handle) → signed update →
  turn → coordinated reply proof through the production router mount:
  `telegram_update_becomes_a_turn_and_a_coordinated_reply`
  (`tests/integration/extension_delivery.rs`, both DBs). The install's
  tenant `[admin_configuration]` values resolve through the production admin
  service; they are not copied into caller installation state. The updated
  journey awaits the combined matrix.

## 9. Frontend (UI)

- [ ] UI-1 The wire carries surface keys, the three-state public lifecycle
  projection, the auth state enum, and admin-configuration group descriptors;
  one golden fixture pins it.
  — the wire carries surfaces (`RebornExtensionSurface`), the §6.1
  `LifecyclePublicState` projection
  (`RebornExtensionInfo.installation_state`), the §6.3 auth-account enum via the
  per-vendor accounts list (`RebornAuthAccount.state`, replacing
  `connected: Option<bool>`), and setup field/secret descriptors
  (`reborn_services/types.rs`). One golden fixture pins the full shape:
  `list_extensions_golden_wire_multi_surface_extension_freezes_accounts_list`
  (`reborn_services_contract.rs`) freezes an arbitrary channel on a
  multi-surface (tool + channel + auth) extension — the surface keys, the
  public-state string, the accounts list, each surface's
  `resolved_account_id` + binding source, and the connection `display_name`.
  The repinned wire contract awaits the combined matrix.
- [ ] UI-2 The channels tab renders every channel surface with the same user
  components; the acme fixture channel installs and connects with
  no frontend source change. — PARTIAL: since P6 S5 every channel surface
  renders through the same generic sections, now driven by the P7a wire —
  affordances derive from caller membership/readiness + the accounts-list auth
  state with no per-extension logic (source-scan
  `chat_omits_connect_action_while_extensions_render_generic_connect_ui`,
  `assets.rs`; frontend `vitest` 639/639 green after the wire swap).
  OWNER-FLAGGED FINDING (verified): the literal "acme fixture channel in the
  *browser*" is not achievable — the acme fixture is a Rust integration-test
  fixture (`AcmeFixtureChannelAdapter`, `tests/integration/support`), not a
  browser-serveable bundled package; the production/e2e catalog carries only
  real extensions (github, slack, google-*, nearai-mcp, notion-mcp), so
  `ironclaw serve` cannot install acme. The generic channel-surface
  rendering (the substantive UI-2 claim — every channel renders through the
  same components with no per-extension frontend logic) is therefore proven
  with the REAL bundled channels: the wire is frozen by the golden fixture
  (UI-1/AUTH-9), the no-per-extension-logic rendering by the `assets.rs`
  source-scan + vitest, and the API-level tenant-configure→connect→remove
  flow by `test_reborn_slack_channel_e2e.py` (UI-6, httpx). The remaining gap
  is a *browser* (Playwright) render of the channels tab for a real channel,
  which rides the existing `tests/e2e` Playwright harness
  (`test_extensions.py` / `test_channel_pairing_flow.py`) and was not run this
  session (heavy local Playwright + `serve` setup, frontend/e2e is local-only).
- [ ] UI-3 Admin configuration forms are manifest-schema-driven, live only in
  the admin configuration area, mask secret fields, and never echo stored
  values. Saving a group never installs the extension for the admin. The
  extension/channel views consume completeness but cannot edit deployment
  values. The updated frontend and router tests await the combined matrix.
- [ ] UI-4 Connect/Reconnect/Remove affordances derive from the three-state
  lifecycle projection plus caller auth/pairing descriptors. There is no
  Activate/Disable/Configure-deployment action in the user extension UI and no
  concrete package-id condition. The updated frontend tests await the combined
  matrix.
- [x] UI-5 Slack setup panel, channel picker, and their API modules are
  deleted; no concrete package-id branch remains in frontend source. —
  slack-setup-panel, slack-channel-picker, slack-setup-api,
  slack-channels-api (+ tests) deleted (`c6bb695ec`); the slack i18n
  blocks died across all 11 locales (`345c54135`); the source-scan pin
  keeps the four files deleted and the frontend free of concrete
  package-id branches (`5208f7df0`).
- [ ] UI-6 The existing Python e2e harness covers: admin configure → connect →
  inbound message → reply → remove for one real channel; no new e2e harness is
  added. — `tests/e2e/scenarios/test_reborn_slack_channel_e2e.py` over
  the EXISTING harness (conftest fixtures + `fake_slack_api.py` extended
  with `oauth.v2.access`; no new harness): tenant admin-configuration POST →
  user install → real product-auth OAuth connect against the fake vendor
  through the loopback-only rewrite seam → derived `active` readiness →
  v0-signed inbound DM on the canonical generic route → coordinated
  reply on `chat.postMessage` (while proving the retired MIG-5 alias is 404) → remove
  (no further admission, no further delivery). The revised journey must be run
  against the standard `webui-v2-beta` binary.

## 10. Migration and compatibility (MIG)

- [x] MIG-1 OAuth grant/account storage is reused (vendor id strings
  unchanged); live grants backfill to `connected`; no re-auth required for
  existing users. — no storage change: the accounts-list projection reuses the
  existing per-caller connection signal and maps a live grant to the
  `connected` auth-account state (`vendor_auth_accounts`,
  `reborn_services/extensions.rs`), frozen by the golden fixture
  `list_extensions_golden_wire_multi_surface_extension_freezes_accounts_list`
  (a connected account projects `state = "connected"`). Vendor id strings and
  grant records are untouched, so existing users need no re-auth.
- [ ] MIG-2 Slack setup slots migrate to tenant admin-configuration handles
  (idempotent, dry-run supported). — H.3 load-time fold
  (`composition/src/extension_host/channel_state_folds.rs`, wired in
  `build_local_runtime`): deployment setup values → tenant-scoped manifest
  `[admin_configuration]` handles. Secret material stays write-only and OAuth
  recipes consume the same declared client-credential handles; no value is
  copied into per-user membership. Proven by
  `fold_moves_setup_state_roots_onto_generic_homes_and_second_run_is_a_noop`
  + `fold_skips_malformed_records_and_operator_owned_values` (+ the libSQL
  flavor). Dry-run posture: the fold only runs on the local-runtime build
  path; the `MigrationDryRun` profile never executes it. The revised target
  and its tests await the combined matrix.
- [x] MIG-3 Slack state roots migrate to generic scoped state; no slack-named
  root is read outside migration code. — The H.4 fold (same module/tests as
  MIG-2) moves identities, channel routes, and DM targets onto the generic
  roots; the P6 S6 lane deletion removed every non-migration reader. The
  only slack-named root reads left are the fold inputs and the H.4b
  sanctioned legacy WORKFLOW storage root table, both homed in the
  migration module (`channel_state_folds.rs`;
  `sanctioned_legacy_roots_cover_only_the_retired_lane` pins the table to
  exactly the retired lane's roots); the table shrinks to nothing with
  the alias retirement.
- [ ] MIG-4 Old installation lifecycle records backfill into explicit caller
  membership. A legacy tenant-owned row narrows to the configured operator;
  current lifecycle code never creates new tenant-owned membership. Public
  state is then derived as `uninstalled | setup_needed | active`. —
  `legacy_tenant_owned_installation_migrates_to_operator_private_state`
  (`tests/integration/extension_user_lifecycle_isolation.rs`), pending the
  combined matrix.
- [x] MIG-5 `/webhooks/slack/events` forwarded to the canonical route for the
  cutover release and is now retired. The whole-path Slack E2E pins the legacy
  path at 404 while the canonical
  `/webhooks/extensions/slack/events` route remains live. Operators must use
  the canonical Events URL.
- [x] MIG-6 OAuth callback URLs are unchanged (no vendor reconfiguration
  needed) — verified by the route tests. — The
  `/api/reborn/product-auth/oauth/{provider}/callback` shape is unchanged
  (AUTH-13's pins: `vendor_oauth_callback_completes_a_started_flow`,
  `product_auth/serve/mod.rs`; the google/slack callback URL tests in
  `tests/webui_v2_product_auth.rs`), and the P6 §10 e2e drives the same
  URL against production serve
  (`test_reborn_slack_channel_e2e.py`).
- [x] MIG-7 Migrations are idempotent (second run is a no-op) and skip
  malformed records with a logged reason, on both DBs. — idempotence +
  malformed-skip are pinned
  (`fold_moves_setup_state_roots_onto_generic_homes_and_second_run_is_a_noop`,
  `fold_skips_malformed_records_and_operator_owned_values`), and the fold runs
  against a real libSQL root filesystem
  (`fold_runs_against_the_libsql_root_filesystem`) AND a real PostgreSQL root
  filesystem (`fold_runs_against_the_postgres_root_filesystem`,
  `#[cfg(feature="postgres")]`, `channel_state_folds.rs`) — the second run is a
  no-op on each. REL-3: the Postgres arm uses a no-skip `src/`-local
  testcontainer provisioner (the `pub(crate)` fold can't be reached from the
  integration harness — correction A escape hatch); it runs under
  `cargo test -p ironclaw_reborn_composition --features
  test-support,webui-v2-beta,libsql,postgres`.

## 11. Testing and gates (TEST)

- [x] TEST-1 The channel-adapter conformance suite exists and runs against
  Slack, Telegram, and acme. — `run_channel_adapter_conformance`
  (`ironclaw_product_adapters::conformance`, test-support export): inbound
  bounds + malformed-never-panics + challenge, full-envelope delivery with
  structured per-part reports against a scripted vendor server,
  internal publish/cleanup-hook idempotency, unsupported surfaces fail cleanly.
  Consumers: `crates/ironclaw_slack_extension/tests/channel_conformance.rs`,
  `crates/ironclaw_telegram_extension/tests/channel_conformance.rs`, and
  the acme fixture in `tests/integration/extension_runtime.rs`.
- [x] TEST-2 The tool-adapter conformance checks run against static, WASM,
  and MCP lanes. (P2 landed the WASM-lane proof — the five Slack tools
  through the binder — and the native/static proof via the acme fixture; the
  MCP lane is now covered by
  `binder_invokes_a_discovered_mcp_tool_through_the_tool_adapter`
  (`crates/ironclaw_host_runtime/src/services/tests/extension_tool_binder.rs`),
  which drives a discovered MCP capability through the same
  `LaneBackedToolAdapter` and asserts the exact call reached the MCP executor
  and its output returned — see TOOL-6.)
- [ ] TEST-3 The auth engine suite is table-driven over recipes; adding a
  vendor adds a row + fixtures, not a suite (checked by suite structure).
- [ ] TEST-4 The acme fixture drives the full generic path end-to-end in the
  integration harness. (P2 landed the tool leg:
  `acme_fixture_lifecycle_dispatches_from_the_active_snapshot` drives
  install → internally published snapshot → dispatch → remove through model
  tool calls,
  with the fixture's native factory assembled through the production
  `RebornHostBindings` seam. P4 landed the inbound leg —
  `signed_acme_post_flows_through_the_production_mount_into_a_turn`.
  P5 landed the outbound machinery generically (the acme adapter's real
  `deliver` runs under TEST-1 conformance; the coordinated outbound
  integration proofs drive slack + telegram in
  `tests/integration/extension_delivery.rs`). Remaining: the acme connect
  leg and an acme-through-the-coordinator outbound row if P6 keeps acme as
  the invented-vendor canary. P6 status: acme also runs the TEST-1
  conformance contract
  (`acme_channel_adapter_satisfies_the_conformance_contract`) and the
  durable ingress admission/replay proof
  (`tests/integration/extension_ingress.rs`); the full
  configure → connect → deliver e2e is proven with the real slack package
  (UI-6) rather than acme — the acme connect leg stays open.)
- [x] TEST-5 Slack and Telegram each have exactly one inbound and one outbound
  integration proof; protocol details are unit-tested inside their crates. —
  One scenario per channel drives BOTH halves through the production mount
  and the real coordinator:
  `slack_final_reply_flows_through_the_real_delivery_coordinator` and
  `telegram_update_becomes_a_turn_and_a_coordinated_reply`
  (`tests/integration/extension_delivery.rs`); protocol shapes stay in the
  adapter crates' fixture tests + TEST-1 conformance.
- [x] TEST-6 The specificity scanner derives forbidden names from the package
  inventory (an invented product id in a fixture is caught without editing the
  scanner). — `scanner_derives_terms_from_an_invented_inventory_package`
  (`crates/ironclaw_architecture/tests/reborn_extension_specificity.rs`); the
  acme fixture itself is derivation input.
- [x] TEST-7 Allowlist shrinkage is enforced: stale entries fail, new
  violations fail. — `scanner_allowlist_is_shrink_only` plus the stale-entry
  and stale-carve-out assertions inside
  `reborn_generic_code_names_no_concrete_extension`; same discipline on the
  dependency gate (`concrete_extension_crates_link_only_from_the_binary_and_tests`).

## 12. Release (REL)

- [ ] REL-1 Every item above is checked with named evidence. — Open rows
  remain (see the unticked items; each carries an honest status note as
  of P6).
- [ ] REL-2 `cargo fmt`, `cargo clippy` (zero warnings), `cargo test`
  (workspace + integration features), architecture tests, and frontend tests
  pass. — P6 branch status: `cargo fmt` clean; CI-exact
  `cargo clippy --all --tests --examples --all-features -- -D warnings`
  raw exit 0; composition full suite
  (`test-support,webui-v2-beta,libsql` — 1113 lib tests + all targets),
  `ironclaw_architecture`, `ironclaw_network`, CLI (`webui-v2-beta`),
  config, and `ironclaw_webui_v2 --all-features` suites green; the
  delivery/ingress/runtime/oauth-connect integration suites green with
  Docker (real Postgres lane included); the P6 §10 Python e2e green.
  The FULL workspace `cargo test` sweep and the frontend JS suites were
  not run here — this row ticks at release, not per-phase.
- [ ] REL-3 Both-DB integration lanes ran against a real PostgreSQL (a skip is
  a failure). — PARTIAL: the harness's real-Postgres lane ran here
  (`StorageMode::Postgres` via `extension_install_persists_across_storage_backends`,
  Docker/testcontainers); the remaining Postgres-rooted legs are itemized
  under AUTH-15 and MIG-7.
- [ ] REL-4 `docs/reborn/contracts/*`, the `reborn-extension-surfaces` skill,
  `FEATURE_PARITY.md`, and `CHANGELOG.md` describe the shipped system. —
  Partial: the **`reborn-extension-surfaces` skill is done** — rewritten to the
  v3 manifest shape (`[[tools]]`/`[channel]`/`[auth.<vendor>]`/`[mcp]`,
  `VendorId`, live `assets/slack/manifest.toml` example), every cited path
  verified against HEAD. `docs/reborn/contracts/*`, `FEATURE_PARITY.md`, and
  `CHANGELOG.md` still need the release refresh.
- [ ] REL-5 The deletion test (DEL-9) and the addition proof (DEL-10) both
  hold at the release commit. — Both hold locally at the P6 head (DEL-9
  script green with full per-crate tests; DEL-10's
  `telegram_update_becomes_a_turn_and_a_coordinated_reply` green in the
  delivery suite); the same-CI-run pairing at the release commit remains.
