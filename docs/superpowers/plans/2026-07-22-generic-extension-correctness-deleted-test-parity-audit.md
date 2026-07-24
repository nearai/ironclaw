# Generic extension correctness: deleted-test parity audit

Status: **inventory complete; replacement execution remains a merge gate; the former “274 deleted tests” claim is disproven**

This audit compares the exact tree merged by PR #6116 with the tree immediately
before that merge. It deliberately treats renamed tests as unproven until a
caller-visible behavior can be mapped to a replacement. A green aggregate test
count is not parity evidence.

## Authoritative comparison

- PR: `nearai/ironclaw#6116`, “feat(reborn): unified generic extension runtime +
  Option A honest state machine (reconcile main)”
- Merge/squash commit: `f7da7dd7b2a928bbe62dd27866fffd2c5b81dd63`
- The merge commit has one parent, so its authoritative pre-merge base is:
  `21bb07df0e92421e368d280ac0bfe8fb433ea403`
- Audited range: `21bb07df0e92421e368d280ac0bfe8fb433ea403..f7da7dd7b2a928bbe62dd27866fffd2c5b81dd63`

The live PR branch name is not an authoritative historical input: its remote
head moved after the merge. The immutable merge tree and its sole parent are.

## Reproducible counting method

The audit archives `crates/`, `tests/`, and `scripts/` at both revisions and
extracts test occurrences by language:

- Rust: `#[test]`, `#[tokio::test]`, `#[async_std::test]`, and `#[rstest]`
  followed by the test function name.
- TypeScript/JavaScript: named `test(...)` and `it(...)` cases, including the
  common `.only`, `.skip`, `.todo`, `.concurrent`, and `.each` forms.
- Python: `def test_*` and `async def test_*`.
- Shell/Bats: named `@test` cases.

“Exact removal” means an `(old path, test name)` occurrence is not present at
that same path and name in the merged tree. “Globally absent” is stricter: the
removed test name does not occur anywhere in the merged tree. This is a lexical
inventory, not an assertion that every renamed test lost its behavior; the
semantic classifications below make that distinction.

| Metric | Result |
|---|---:|
| Test occurrences before | 15,136 |
| Test occurrences after | 14,712 |
| Net | -424 |
| Exact `(path, name)` removals | 1,024 |
| Exact `(path, name)` additions | 600 |
| Removed names absent everywhere after merge | 953 |
| Tests in files deleted outright | 685 across 79 test-bearing files |
| Globally absent Rust names | 898 |
| Globally absent TypeScript names | 50 |
| Globally absent TSX names | 3 |
| Globally absent Python names | 2 |

Therefore the checklist’s earlier “274 directly deleted tests” number is not a
valid count for the authoritative merge range under any of the documented
measures. It must not be used as a sign-off denominator.

## Semantic risk classification

### Preserved

Tests whose exact path and symbol survive are mechanically preserved and are
not included in the 1,024 removals. This category alone says nothing about
weakened assertions; the merge-readiness checklist still requires review of
changed test bodies at load-bearing seams.

### Re-expressed through generic owners

The following mappings have a concrete new owner and caller-visible assertion.
The new test is not required to retain a vendor-specific name.

| Behavior | Old path / exact symbol | New path / exact symbol | Classification |
|---|---|---|---|
| delayed OAuth completion still reaches source channel | `crates/ironclaw_channel_delivery/src/tests.rs::triggered_persistent_blocked_oauth_auth_records_delivered_not_failed` and related timeout tests | `crates/ironclaw_reborn_composition/src/extension_host/channel_host/e2e_tests.rs::external_channel_delivers_final_after_oauth_outlives_delivery_poll_window` | re-expressed at composed channel seam |
| duplicate delayed auth lifecycle events send once | channel-delivery duplicate/guard cases in `crates/ironclaw_channel_delivery/src/tests.rs` | `crates/ironclaw_product_workflow/tests/run_delivery_contract.rs::duplicate_lifecycle_events_deliver_once_after_delayed_oauth` | re-expressed generically |
| revoked channel blocks delayed delivery | vendor/channel-delivery target-revalidation cases | `crates/ironclaw_product_workflow/tests/run_delivery_contract.rs::channel_removal_or_unpairing_revokes_delayed_delivery` | re-expressed generically |
| trigger result uses creator-selected external target | `crates/ironclaw_channel_delivery/src/tests.rs::driver_fire_with_delivery_target_routes_to_it_over_the_preference` | `tests/integration/group_triggers/scenario_external_source_trigger_captures_delivery.rs::run` and `tests/integration/delivery_user_journeys.rs::scheduled_routine_persists_the_exact_listed_target` | re-expressed through normal trigger/capability path |
| per-user extension install/remove isolation | bespoke Slack installation/connection tests | `tests/integration/extension_user_lifecycle_isolation.rs::users_install_and_remove_the_same_extension_independently` | re-expressed through production WebUI facade |
| admin configuration does not install an extension | bespoke Slack setup/catalog tests | `tests/integration/extension_user_lifecycle_isolation.rs::admin_configuration_does_not_install_an_extension_for_any_user` | re-expressed generically |
| operator personal installation is ordinary user state | old operator-scoped Slack setup/visibility tests | `tests/integration/webui_v2_product_api.rs::users_and_operator_install_and_remove_independently_through_production_webui_facade` | re-expressed through signed caller path |
| repeated admin secret paste survives refetch | `crates/ironclaw_webui/frontend/src/components/slack-setup-panel.test.ts::SlackSetupPanel does not reset dirty form fields on background setup refetch` | `crates/ironclaw_webui/frontend/src/pages/admin/components/configuration-tab.test.ts::configuration group keeps repeated secret pastes mounted and dirty across a manifest refetch`; `tests/e2e/scenarios/test_admin_api.py::test_admin_configuration_repeated_paste_keeps_form_mounted` | re-expressed in generic admin UI plus browser E2E |
| deep hosted-MCP discovery publishes the catalog | older shallow/hosted MCP discovery assertions | `crates/ironclaw_mcp/tests/mcp_adapter_contract.rs::concrete_mcp_http_client_discovers_bounded_deep_openapi_schema` | re-expressed through concrete client |
| pairing mint, consume, repair, and cleanup | `crates/ironclaw_telegram_extension/src/pairing/tests.rs::{issue_mints_code_with_deep_link_and_ttl,consume_happy_path_binds_targets_and_dispatches,same_user_re_pair_is_idempotent,unpair_removes_binding_target_and_pending_code}` | `crates/ironclaw_reborn_composition/src/extension_host/channel_pairing/tests.rs::{mint_rotates_to_a_single_live_code_and_resolves_the_deep_link,consume_binds_identity_records_dm_target_then_dispatches_continuation,bound_sender_rerunning_a_code_repairs_completion_idempotently,unpair_drops_bindings_target_codes_and_conversation_actor_pairings}` | re-expressed generically |
| generic Slack/Telegram ingress, auth prompt, final reply, and trigger delivery | `crates/ironclaw_reborn_composition/src/slack/slack_host_beta.rs` and deleted Telegram ingress suites | `crates/ironclaw_reborn_composition/src/extension_host/channel_host/e2e_tests.rs::{slack_dm_for_personally_bound_user_routes_through_reborn_identity,shared_channel_admission_follows_saved_admin_configuration,auth_resolution_and_lifecycle_events_deliver_each_stage_once,generic_triggered_hook_routes_fire_to_the_owning_extension_driver}` plus both extension `channel_conformance.rs` suites | re-expressed through generic host and adapter conformance |

### Ratified removals

These tests asserted an intentionally retired architecture rather than a
surviving user behavior. Their removal is acceptable only because the retired
surface is also pinned absent by architecture tests.

| Retired behavior | Old path / symbols | Ratification |
|---|---|---|
| separate `ironclaw_channel_delivery` crate/public API | `crates/ironclaw_channel_delivery/src/tests.rs` (91 tests) and `tests/public_api.rs::public_api_exposes_expected_delivery_types` | orchestration moved to product workflow; dependency/retired-taxonomy gates prohibit the old rail |
| standalone WASM product-adapter runtime | `crates/ironclaw_wasm_product_adapters/tests/component_runtime_contract.rs` and in-module runtime tests | product adapters now implement the channel contract; no compatibility runtime remains |
| v1-to-Reborn migration executable and its legacy snapshot fixtures | `crates/ironclaw_reborn_migration/tests/migration_roundtrip.rs` and in-module migration tests | whole migration crate removed from the workspace; not a current runtime path |
| Slack-only connectable-channel API and UI | `slack-channel-picker.test.tsx`, `slack-channels-api.test.ts`, and Slack connectable/setup symbols | extension surfaces and generic admin configuration replace the parallel registry |
| explicit legacy activation assertions | deleted `activate_then_active`/vendor activation assertions that require a second user action after auth | current contract derives `uninstalled -> setup_needed -> active`; no public Activate transition |

This category does **not** ratify loss of ingress authentication, caller
isolation, target revalidation, OAuth continuation, delivery evidence, or
pairing concurrency merely because their original test lived in a retired
crate.

### Regressions added after the initial audit

The initial audit found three concrete caller-path gaps. Replacement journeys
are now present in the working tree. This is source-level parity evidence only:
the final exact-head test matrix must still execute them successfully before
merge.

| Required journey | Exact deleted evidence | Replacement now present | Caller-visible proof |
|---|---|---|---|
| fresh-thread unpair then re-pair | `tests/integration/telegram_journeys/scenario_unpair_repair_fresh_slate.rs::telegram_unpair_then_repair_starts_fresh_thread_not_the_old_blocked_one` | `tests/integration/extension_delivery.rs::unbound_telegram_actor_pairs_via_web_minted_code_then_turns_attribute_to_the_paired_user` | production HTTP unpair plus verified Telegram webhook re-pair; the same external actor resolves to a new thread and the repaired reply is delivered |
| exactly one concurrent pairing-code winner plus wrong-user isolation | `crates/ironclaw_telegram_extension/src/pairing/tests.rs::concurrent_consume_of_one_code_binds_exactly_one_winner` and `telegram_account_bound_to_other_user_is_refused` | `crates/ironclaw_reborn_composition/src/extension_host/channel_pairing/tests.rs::{concurrent_caller_admission_has_exactly_one_pairing_winner,caller_admission_isolates_foreign_installations_and_wrong_users}` | two concurrent ingress admissions yield one durable binding; foreign-installation and already-bound wrong-user attempts reveal no code ownership and preserve the existing binding |
| permanent send failure terminates and later delivery recovers without duplicates | deleted vendor delivery failure/retry cases, including `crates/ironclaw_channel_delivery/src/tests.rs::driver_slack_api_rejection_records_failed_not_delivered` | `crates/ironclaw_product_workflow/tests/run_delivery_contract.rs::permanent_channel_failure_is_terminal_across_later_recovery` | the failed run records terminal evidence and is not resent during recovery; a later independent run reaches the provider once and records delivered evidence |

## Deleted high-risk inventory by old path

This table is the review routing ledger for the largest globally absent groups.
Counts are exact removed names absent anywhere in the merged tree, not line
deletions.

| Old path | Absent names | Semantic owner after refactor | Verdict |
|---|---:|---|---|
| `crates/ironclaw_channel_delivery/src/tests.rs` | 91 | `ironclaw_product_workflow` run delivery + composed channel E2E | mixed: re-expressed, with permanent-recovery gap above |
| `crates/ironclaw_reborn_composition/src/slack/slack_host_beta.rs` | 49 | generic channel host, outbound registry, adapter conformance | re-expressed by behavior; vendor topology retired |
| `crates/ironclaw_reborn_composition/src/slack/slack_host_state.rs` | 36 | extension host deployment/configuration stores | re-expressed/ratified vendor-store removal |
| `crates/ironclaw_reborn_composition/src/product_auth/serve/oauth.rs` | 27 | `ironclaw_auth` engine + product-auth API/serve adapters | re-expressed; callback/blocked-gate whole paths remain mandatory |
| `crates/ironclaw_reborn_composition/src/slack/slack_outbound_targets.rs` | 24 | mutable generic outbound registry + generic channel target provider | re-expressed generically |
| `crates/ironclaw_auth/tests/auth_product_contract/oauth_helpers_contract.rs` | 22 | split auth-engine/provider contract suites | re-expressed across owner modules |
| `crates/ironclaw_reborn_composition/src/slack/slack_channel_routes.rs` | 21 | generic admin configuration and subject-route store | re-expressed generically |
| `crates/ironclaw_reborn_composition/src/product_auth/durable/tests.rs` | 20 | split durable auth modules/tests | review by auth contract owner; not ratified wholesale |
| `crates/ironclaw_wasm_product_adapters/src/auth_verifier.rs` | 20 | channel adapter/host auth boundary | retired runtime; security behavior must remain in generic conformance |
| `crates/ironclaw_telegram_extension/src/setup/tests.rs` | 19 | manifest admin configuration + generic pairing | re-expressed by generic lifecycle; no Telegram admin form in user UI |
| `crates/ironclaw_telegram_extension/src/ingress/tests.rs` | 17 | generic extension ingress + Telegram conformance | re-expressed; webhook auth/limits still require caller-level coverage |
| `crates/ironclaw_reborn_composition/src/slack/slack_channel_routes/allowed/tests.rs` | 16 | generic subject-route/admission policy | re-expressed generically |
| `crates/ironclaw_reborn_composition/src/product_auth/oauth/oauth_provider_client/tests.rs` | 16 | `ironclaw_auth` concrete engine/client contracts | re-expressed by owner crate |
| `crates/ironclaw_telegram_extension/src/ingress/dispatch_tests.rs` | 15 | generic ingress router and channel-host E2E | re-expressed generically |
| `crates/ironclaw_webui/frontend/src/pages/extensions/components/channels-tab.test.ts` | 15 | same generic Channels tab after surface/lifecycle rewrite | changed-body review required; not a ratified deletion |
| `crates/ironclaw_reborn_composition/src/slack/slack_channel_connection.rs` | 15 | generic channel connection/pairing lifecycle | re-expressed generically |
| `crates/ironclaw_{slack,telegram}_v2_adapter/src/adapter.rs` | 33 combined | extension channel conformance suites | re-expressed at contract seam |
| `crates/ironclaw_wasm_product_adapters/tests/component_runtime_contract.rs` | 14 | no replacement runtime | ratified architectural removal |
| `crates/ironclaw_telegram_extension/src/pairing/tests.rs` | 13 | generic channel pairing service | mixed: two missing caller/concurrency journeys above |
| `crates/ironclaw_reborn_migration/tests/migration_roundtrip.rs` | 10 | none | ratified whole-crate retirement |

## Architecture conformance discovered during the audit

The current correctness work initially introduced a second delivery-target ID
wrapper in `ironclaw_outbound` while triggers retained a 256-byte mirror. Both
consumer crates already depend on neutral `ironclaw_host_api`, so the correct
shape is one canonical `OutboundDeliveryTargetId` there:

- 512-byte maximum;
- no empty/whitespace-only or surrounding whitespace;
- no control, line/paragraph separator, or unsafe Unicode formatting
  characters;
- canonical serde once, re-exported by `ironclaw_outbound` and aliased as
  `TriggerDeliveryTargetId` by `ironclaw_triggers`;
- trigger-only error adaptation occurs at trigger ingestion/repository
  boundaries as `TriggerError::InvalidRecord(DeliveryTargetInvalid)`.

The target routing abstraction audit also found:

- `RouteCurrentRunFinalReply` is justified: host runtime consumes a narrow
  product-owned mutation port and cannot depend on product-workflow concrete
  services without reversing the dependency direction.
- `TriggerFinalReplyTargetAuthority` was not justified: it had one production
  implementation and bundled turn state, outbound state, and target lookup
  behind a composition-shaped umbrella. It is removed. The service now depends
  directly on `TurnStateStore`, `OutboundStateStore`, and the existing
  runtime-polymorphic `CurrentDeliveryTargetResolver`.
- `CurrentDeliveryTarget` carries its owning extension id so replay/fanout can
  prove handler ownership without parsing provider-specific target strings.

These changes follow `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`:
one shared contract type, fewer mirror DTOs, no one-implementation domain
`dyn` shim, and product policy outside the composition crate.

## Merge gate

This audit is complete as an inventory, but parity is **not green** until:

1. the three replacement journeys above pass on the exact final PR SHA;
2. all re-expressed rows pass through their production caller seam;
3. the final PR SHA passes the deterministic matrix in the merge-readiness
   checklist; and
4. no review weakens an assertion merely to match current behavior.
