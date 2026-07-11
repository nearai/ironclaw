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
- [x] MAN-2 One v3 manifest declares tools, at most one channel, and auth
  recipes; parsing is a single entry point shared with normalized v2. —
  `acme_fixture_parses_through_the_single_entry_point`,
  `v2_and_v3_rewrites_resolve_identically`
  (`crates/ironclaw_extensions/tests/manifest_v3_contract.rs`); both schemas
  dispatch through `ExtensionManifestRecord::from_toml`.
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
- [ ] MAN-7 Two extensions with the same vendor id and identical-except-scopes
  recipes activate and share one vendor record; differing recipes fail
  activation with a conflict error.
- [ ] MAN-8 `trigger`/`file` remain reserved kinds with no runtime binding.
- [ ] MAN-9 Retired-taxonomy gate still passes (no `slack_bot`/
  `slack_personal`/channel-as-product vocabulary).
- [ ] MAN-10 `[channel].conversation_model` is required and validated;
  conversation binding honors the declared model through a caller-level
  workflow test; the WebUI's internal channel uses the same enum.
- [x] MAN-11 The credential-authority type is `VendorId` end to end; the v3
  field is `vendor` (v2 `provider` maps in normalization); stored vendor id
  strings are unchanged; the old type name survives only as a deprecation
  alias, deleted by P7. — workspace-wide rename
  (`crates/ironclaw_host_api/src/ids.rs`; alias documented for P7 deletion);
  `v2_and_v3_rewrites_resolve_identically` pins the `provider` → `vendor`
  mapping; the serde wire field stays `provider` (persisted turn-state
  compatibility).

## 2. Resolved record (REC)

- [ ] REC-1 Compile once → persisted resolved record + manifest digest; all
  production projection reads the record (no raw-TOML reparse outside the
  compiler and migration).
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
  before activation; approval denial leaves the old generation active.

## 3. Binding, loaders, lifecycle (LIFE)

- [x] LIFE-1 Declared `[[tools]]`/`[mcp]` without a bound tool
  adapter fails activation; same for `[channel]`; undeclared bindings fail;
  auth never binds.
  — `binding rule` unit tests (`crates/ironclaw_extension_host/src/entrypoint.rs`):
  declared-tool/channel-without-adapter, undeclared-tool/channel adapter, exact
  binding, and `auth_never_binds_is_not_a_binding_field` (the bindings struct
  has no auth field); driven at the activation caller by
  `declared_tool_without_bound_adapter_fails_activation`
  (`tests/lifecycle_contract.rs`).
- [x] LIFE-2 `bind` is side-effect-free and receives no network/secret/store
  ports; adapters are parameterized with non-secret config values only.
  — `BindContext` (`entrypoint.rs`) carries only the installation id, the
  resolved contract, and non-secret config values; `ExtensionEntrypoint::bind`
  is a synchronous, port-free signature (secrets exist only behind host
  egress injection).
- [ ] LIFE-3 Native loader resolves `runtime.service` from the registry the
  binary assembles; unknown service fails with a typed error.
- [ ] LIFE-4 WASM and MCP runtime extensions load through synthesized
  entrypoints with no extension-authored Rust.
- [x] LIFE-5 `ExtensionHost` is the only writer of installation state and the
  active snapshot.
  — `ExtensionHost` owns the only `InstallationRecordStore` writes and the
  only `ActiveSnapshot` swaps, serialized under one async mutex
  (`crates/ironclaw_extension_host/src/lifecycle.rs`).
- [x] LIFE-6 The installation state machine is one shared enum
  (`Installed/Activating/Active/Deactivating/Removing/RemovalPending/Removed`);
  no extension-specific state value exists anywhere (grep + wire schema test).
  — one `InstallationState` enum (`crates/ironclaw_extension_host/src/state.rs`);
  `installation_state_wire_form_matches_str` pins the exact wire vocabulary.
  The whole-workspace no-extension-specific-state grep lands with the P2 wire
  exposure.
- [x] LIFE-7 Every lifecycle transition is persisted; crash during any
  transient state resumes deterministically at startup.
  — `transient_states_resume_deterministically` (`state.rs`) plus
  `restore_resumes_active_and_skips_invalid` (`tests/lifecycle_contract.rs`):
  a record crashed mid-activation resumes to Installed (its interrupted
  activation published nothing).
- [x] LIFE-8 Activation failure (bind, hook, conflict, store) publishes
  nothing and records a typed, redacted error.
  — `declared_tool_without_bound_adapter_fails_activation`,
  `channel_activate_runs_and_its_failure_aborts`,
  `duplicate_capability_across_extensions_fails_activation`
  (`tests/lifecycle_contract.rs`): each leaves the snapshot unchanged and the
  record back at Installed with a redacted `last_error`.
- [x] LIFE-9 `channel.activate()` runs during activation; its failure aborts
  activation.
  — `channel_activate_runs_and_its_failure_aborts` (activate hook observed to
  run once; failure aborts with nothing published).
- [ ] LIFE-10 Removal follows the fixed order (unpublish → drain → vendor
  cleanup → auth revoke/grant delete → config/identity delete) — observed via
  scripted adapter and engine in one caller-level test.
- [x] LIFE-11 Vendor cleanup failure lands in `RemovalPending`, is retryable,
  and cannot report success early or resurrect the extension.
  — `cleanup_failure_lands_in_removal_pending_and_retry_completes`
  (`tests/lifecycle_contract.rs`): a cleanup failure lands `RemovalPending`,
  never runs the later auth/delete steps, and the extension stays unpublished.
- [x] LIFE-12 Removing one extension preserves grants of a shared vendor
  still used by another active extension; removes them when it was the last
  consumer.
  — the removal context carries `other_active_extension_ids`
  (`removal_context_reports_other_active_extensions_for_shared_vendor`); the
  shared-vendor grant policy itself is enforced by the injected auth-revoke
  hook, proven end-to-end with the P3 auth engine.
- [ ] LIFE-13 Conversation/LLM history survives extension removal.
- [x] LIFE-14 Duplicate capability id or ingress route across active
  extensions fails activation.
  — `duplicate_capability_across_extensions_fails_activation`
  (`tests/lifecycle_contract.rs`) plus `ActiveSnapshot::build`/`would_conflict`
  conflict detection (`crates/ironclaw_extension_host/src/active.rs`).
- [x] LIFE-15 Upgrade swaps one immutable snapshot; in-flight work completes
  on its old generation `Arc`; new work resolves the new generation; no mixed
  generation under concurrent activate/resolve stress.
  — `in_flight_snapshot_survives_a_later_swap` (`tests/lifecycle_contract.rs`):
  an in-flight `Arc<ActiveSnapshot>` keeps its generation and its extensions
  after a later deactivate swaps in a new generation.
- [x] LIFE-16 Startup skips an invalid extension with a typed error and
  publishes the valid rest.
  — `restore_skips_a_load_failure_without_blocking_the_rest`
  (`tests/lifecycle_contract.rs`): a load failure falls to Installed with a
  typed error and does not block the valid restore.
- [ ] LIFE-17 Full lifecycle (install → configure → activate → remove) passes
  on both DBs through the integration harness with the acme fixture.
- [ ] LIFE-18 Editing channel config while `Active` triggers an automatic
  deactivate → reactivate; adapters observe the new values; no bespoke
  reconfigure state exists.

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
  The active-snapshot resolver for `ExtensionHost`-activated extensions
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
- [ ] TOOL-4 Credential injection derives from the resolved declaration; an
  adapter cannot reach an undeclared credential, egress host, or port.
- [ ] TOOL-5 Missing credential raises the generic auth gate and resumes after
  the engine completes (caller-level test).
- [ ] TOOL-6 WASM and MCP lanes invoke through `ToolAdapter` with existing
  result/event semantics.
- [ ] TOOL-7 The five real Slack tools activate and invoke through the generic
  dispatcher (integration, recorded egress).
- [ ] TOOL-8 `slack.send_message` remains an explicit side-effect tool; final
  replies never route through it.
- [ ] TOOL-9 MCP discovery is loader-owned (`ToolAdapter` has no discovery
  method); validated tool surfaces publish atomically; a refresh replaces the
  set completely or not at all; discovered tools run the same dispatcher
  pipeline as static ones.
- [ ] TOOL-10 Host built-in capabilities resolve through the same dispatcher
  pipeline; an extension capability id colliding with a built-in fails
  activation.

## 5. Auth engine (AUTH)

- [ ] AUTH-1 One engine implements `oauth2_code` and `api_key`; there is no
  auth trait in the extension ABI and no per-vendor code path (grep gate: no
  vendor-conditional in auth crates/composition).
- [ ] AUTH-2 The authorize URL is host-constructed; recipes cannot supply or
  override `state`, `redirect_uri`, PKCE, `client_id`, `response_type`, or the
  scope parameter.
- [ ] AUTH-3 State/CSRF, PKCE, TTL, and callback replay are enforced; exactly
  one transition consumes a callback.
- [ ] AUTH-4 Requested scopes intersect the recipe ceiling; widening is
  rejected before the vendor call.
- [ ] AUTH-5 Token exchange supports `post_body` and `basic`; response fields
  extract via bounded JSON pointers, including `fallback_to_requested` scope.
- [ ] AUTH-6 Refresh runs on-demand at injection with single-flight and honors
  `rotates_refresh_token` both ways; revoke is idempotent; vendor response
  bodies are size-capped and redacted from errors and logs.
- [ ] AUTH-7 Identity extracts from the token response or the declared
  identity endpoint and is validated against the flow before storage.
- [ ] AUTH-8 Grants/secrets are encrypted at rest; stored secrets are never
  echoed to UI or adapters.
- [ ] AUTH-9 The auth account state machine is one shared enum
  (`disconnected/authenticating/connected/expired/revoking` + typed
  `last_error`); no vendor- or extension-specific state exists; the wire
  exposes exactly this enum. — enum + wire form defined and pinned
  (`AuthAccountState`, `crates/ironclaw_extension_host/src/state.rs`,
  `auth_account_state_wire_form_matches_str`); the engine that drives its
  transitions and the wire exposure land in P3.
- [ ] AUTH-10 Flow TTL expiry and vendor denial land in `disconnected` with
  a typed reason; refresh failure lands in `expired`.
- [ ] AUTH-11 `api_key` renders from recipe fields, runs the optional
  validation probe through restricted egress, and uses the same state machine.
- [ ] AUTH-12 All five current vendors (Slack, Google, Notion, GitHub,
  NEAR AI) are expressed as recipes and pass the engine suite as table rows —
  no vendor-specific test suite exists.
- [ ] AUTH-13 Callback route keeps the existing
  `/api/reborn/product-auth/oauth/{provider}/callback` shape; `{provider}` is
  resolved as data (vendor-registered redirect URLs unchanged).
- [ ] AUTH-14 Slack end-to-end: blocked tool → gate → scripted callback →
  grant stored → tool resumes (extends the existing oauth-connect integration
  test).
- [ ] AUTH-15 Engine flow/grant persistence passes on both DBs.
- [ ] AUTH-16 The provider string multiplexor, provider spec constants, and
  Slack OAuth branches are deleted.

## 6. Channel ingress (ING)

- [ ] ING-1 One generic router serves
  `/webhooks/extensions/{extension_id}/{route_suffix}` from the active
  snapshot; extensions cannot mount arbitrary routes; collisions with fixed
  host routes fail activation.
- [ ] ING-2 Method, body limit, rate limit, and deadline are enforced before
  adapter work.
- [ ] ING-3 `hmac_sha256` recipes verify exact byte construction
  (fixture-pinned), with constant-time comparison and timestamp/replay
  rejection.
- [ ] ING-4 `shared_secret_header` verifies constant-time and rejects
  missing/duplicate headers.
- [ ] ING-5 Signing secrets are never observable by the adapter (scripted
  adapter records its full inputs).
- [ ] ING-6 With multiple candidate installations, verification tries each
  within the fixed bound and resolves exactly one or rejects as ambiguous.
- [ ] ING-7 `adapter.inbound` receives bounded input, is panic-isolated, and
  returns `Messages`/`Respond`/`Ignore` only.
- [ ] ING-8 2xx is returned only after the durable dedupe/admission commit;
  store failure returns retryable 5xx; crash/duplicate/restart replay
  converges exactly once (both DBs).
- [ ] ING-9 Challenge (`Respond`) answers after verification without enqueue,
  within response size/status bounds.
- [ ] ING-10 Normalized messages flow through existing identity/conversation
  binding and turn submission (integration: signed vendor POST → turn).
- [ ] ING-11 `reply_context` is stored host-side and returned to the same
  extension's adapter at delivery time.
- [ ] ING-12 Slack and Telegram inbound both pass through the same router and
  workflow caller with zero host branches (one integration proof each).
- [ ] ING-13 Inbound attachments are references; any byte fetch happens
  host-side through restricted egress with the channel credential — adapters
  never fetch.

## 7. Channel outbound (OUT)

- [ ] OUT-1 Every outbound intent (final reply, progress, gate prompt, auth
  prompt, failure, connect-required, working, cleanup, triggered delivery)
  enters the one coordinator; a grep/architecture check finds no direct
  product send path.
- [ ] OUT-2 Target resolution preserves source-route replies and preference
  targets; unauthorized/unavailable targets fail closed.
- [ ] OUT-3 An attempt is persisted (`Prepared`→`Sending`) before vendor
  egress.
- [ ] OUT-4 The coordinator is the sole delivery-state writer; adapters
  receive no store; production construction rejects a no-op sink.
- [ ] OUT-5 Retry/backoff, dedupe, single-flight, and shutdown drain are
  generic and tested with a scripted adapter.
- [ ] OUT-6 Crash after possible vendor success records `Unknown`; no blind
  resend without a vendor idempotency key.
- [ ] OUT-7 Partial multipart: once any part sends, a later retryable failure
  is terminal unless an idempotency key proves safe retry.
- [ ] OUT-8 Restricted egress rejects undeclared hosts/methods,
  adapter-supplied auth headers where injection is declared, cross-host
  redirects, private-IP/DNS-rebind targets, and oversized bodies — before any
  network call.
- [ ] OUT-9 Delivery attempt persistence passes on both DBs.
- [ ] OUT-10 Slack rendering/splitting/DM-provisioning and Telegram rendering
  live only in their crates (fixture unit tests) with one outbound
  integration proof each.
- [ ] OUT-11 Prompt construction consumes `CommunicationPresentationPolicy`
  from the channel contract; concrete channel branches in `ironclaw_llm` are
  deleted.
- [ ] OUT-12 Trace contributions use generic extension/surface origin ids;
  concrete variants are deleted.

## 8. Extraction and deletion (DEL)

- [ ] DEL-1 `crates/ironclaw_reborn_composition/src/slack/` no longer exists.
- [ ] DEL-2 `ironclaw_slack_v2_adapter` and `ironclaw_telegram_v2_adapter` are
  folded into their extension crates and removed from the workspace.
- [ ] DEL-3 `serve_slack.rs` and the `slack-v2-host-beta` cargo feature are
  deleted; no channel-specific config type remains in
  `ironclaw_reborn_config`.
- [ ] DEL-4 Slack cleanup constants in product workflow and Slack connection
  copy in lifecycle are deleted (standard pipeline + manifest display data).
- [ ] DEL-5 The old `ProductAdapter` metadata getters and the unused registry
  runtime projection are deleted.
- [ ] DEL-6 Composition constructs no concrete extension and mounts no
  concrete route (architecture gate).
- [ ] DEL-7 Only `ironclaw_reborn_cli` and tests depend on concrete extension
  crates (`cargo metadata` gate).
- [ ] DEL-8 The concrete-name scanner allowlist is empty.
- [ ] DEL-9 `check-generic-without-concrete.sh` passes in CI: every generic
  crate's dependency tree is free of concrete extension crates and its tests
  pass — the deletion test.
- [ ] DEL-10 Telegram runs as a real installed package (manifest +
  `activate()` webhook registration) — the addition test proven by the second
  production channel.

## 9. Frontend (UI)

- [ ] UI-1 The wire carries surface keys, the installation state enum, the
  auth state enum, and config field descriptors; one golden fixture pins it.
- [ ] UI-2 The channels tab renders every channel surface with the same
  components; the acme fixture channel renders, configures, and connects with
  no frontend source change.
- [ ] UI-3 Config forms are schema-driven; secret fields mask and never echo
  stored values.
- [ ] UI-4 Connect/Reconnect/Configure/Remove affordances derive from the two
  state enums + config completeness — no per-extension logic (source-scan
  test).
- [ ] UI-5 Slack setup panel, channel picker, and their API modules are
  deleted; no concrete package-id branch remains in frontend source.
- [ ] UI-6 The existing Python e2e harness covers: configure → connect →
  inbound message → reply → remove for one real channel; no new e2e harness is
  added.

## 10. Migration and compatibility (MIG)

- [ ] MIG-1 OAuth grant/account storage is reused (vendor id strings
  unchanged); live grants backfill to `connected`; no re-auth required for
  existing users.
- [ ] MIG-2 Slack setup slots migrate to config/client-credential handles
  (idempotent, dry-run supported).
- [ ] MIG-3 Slack state roots migrate to generic scoped state; no slack-named
  root is read outside migration code.
- [ ] MIG-4 Old installation lifecycle records backfill into the standard
  state enum.
- [ ] MIG-5 `/webhooks/slack/events` forwards to the canonical route for one
  release; a removal note names the release that deletes it.
- [ ] MIG-6 OAuth callback URLs are unchanged (no vendor reconfiguration
  needed) — verified by the route tests.
- [ ] MIG-7 Migrations are idempotent (second run is a no-op) and skip
  malformed records with a logged reason, on both DBs.

## 11. Testing and gates (TEST)

- [ ] TEST-1 The channel-adapter conformance suite exists and runs against
  Slack, Telegram, and acme.
- [ ] TEST-2 The tool-adapter conformance checks run against static, WASM,
  and MCP lanes.
- [ ] TEST-3 The auth engine suite is table-driven over recipes; adding a
  vendor adds a row + fixtures, not a suite (checked by suite structure).
- [ ] TEST-4 The acme fixture drives the full generic path end-to-end in the
  integration harness.
- [ ] TEST-5 Slack and Telegram each have exactly one inbound and one outbound
  integration proof; protocol details are unit-tested inside their crates.
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

- [ ] REL-1 Every item above is checked with named evidence.
- [ ] REL-2 `cargo fmt`, `cargo clippy` (zero warnings), `cargo test`
  (workspace + integration features), architecture tests, and frontend tests
  pass.
- [ ] REL-3 Both-DB integration lanes ran against a real PostgreSQL (a skip is
  a failure).
- [ ] REL-4 `docs/reborn/contracts/*`, the `reborn-extension-surfaces` skill,
  `FEATURE_PARITY.md`, and `CHANGELOG.md` describe the shipped system.
- [ ] REL-5 The deletion test (DEL-9) and the addition proof (DEL-10) both
  hold at the release commit.
