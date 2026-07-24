# Telegram QA-catalog coverage map

Machine-checked inventory of every Telegram row in the manual-QA catalog
(`~/ironclaw-manual-qa/catalog/*.json` — the local ground truth; the Notion
"IronClaw Test Coverage" page is a projection of it) against the automated
suites on PR #6159. **Row selection:** a row counts as Telegram if its id is
telegram-prefixed (`qa-telegram:*`, `derived:telegram:*`, `*:telegram:*`) or
its `surfaces`/`scope`/`coverage_tags` name telegram — 160 rows at capture
time (95 Hermetic Integration).

Statuses: **covered** (a named automated test pins the behavior),
**partial** (parts pinned; the note names the uncovered leg), **needs-test**
(automatable at a deterministic tier, not yet written), **gap-product** (the
row describes behavior that is NOT IMPLEMENTED — per the owner rule, no test
was faked), **divergence** (shipped behavior deliberately differs from the
row draft — owner adjudication requested), **draft** (catalog placeholder
with no executable sub-rows), **not-automated** (wrong tier for hermetic
automation; reason given).

Regenerate the row inventory: filter the catalog as above; test evidence is
by exact test name — `cargo test <name>` locates each.

## Hermetic Integration (95 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `qa-install:C6:06:channel-recipe-readiness`<br>Bulk install registers channel requirements without exposing them to members | **needs-test** | Bulk-install recipe readiness (channel requirements registered but not member-exposed) — owned by the extension-recipes suites, not the telegram host; needs triage against the recipe/readiness tests in extension_host (not attempted tonight). |
| `qa-multitenant-setup:J5:03:telegram-asymmetric-binding-fixture`<br>User A pairs with Telegram while an unknown provider user remains unbound | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_two_users_stay_isolated_across_pairing_reply_and_unpair (paired user + unbound provider identity coexist) |
| `qa-remove-reconfigure:RA-14`<br>Actorless removal is rejected only when owner-scoped cleanup is required | **needs-test** | Actorless-removal arbitration (rejected only when owner-scoped cleanup is required) — lives in extension_lifecycle removal paths; needs a targeted case there (not attempted tonight). |
| `qa-remove-reconfigure:RA-7`<br>A missing Telegram member-cleanup adapter blocks deletion and a restored adapter completes retry | **covered** | composition suite (`--lib`) extension_removal_cleanup::{registry_fails_closed_for_unknown_required_adapter, telegram_removal_cleanup_fails_closed_when_facade_slot_is_unfilled} |
| `qa-remove-reconfigure:RA-7:02:adapter-error-recovery`<br>A failing Telegram member-cleanup adapter preserves state and a healthy retry removes it | **partial** | composition suite (`--lib`) extension_removal_cleanup::registry_sanitizes_adapter_failures (failure preserves state + sanitized error); the healthy-retry-completes leg needs a two-phase case in the same module. |
| `qa-remove-reconfigure:RC-2:02:telegram-token-rollback`<br>A Telegram webhook-registration failure preserves the prior working admin configuration | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_setup::{rollback_restores_previous_record_and_previous_webhook_registration, rollback_after_bot_swap_deletes_the_new_bots_webhook, failed_secret_persist_deletes_fresh_webhook_when_no_previous} |
| `qa-routines:RA5:02:unpaired-telegram-recovery`<br>An unpaired Telegram target receives nothing until the next slot after pairing | **partial** | Unpaired target ⇒ empty provider list (`cargo test -p ironclaw_telegram_extension` telegram_outbound_targets::list_is_empty_when_caller_is_unpaired) + generic driver skip; the timed next-slot-after-pairing walk needs the routine poller clock — Runtime Integration shape. |
| `qa-slack:E17`<br>A Slack payload cannot authenticate or dispatch through Telegram ingress | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_handler_rejects_foreign_channel_payload_without_turn (telegram leg; the slack-side leg lives with the slack suites) |
| `qa-telegram:A1`<br>An unconfigured Telegram webhook rejects valid updates before adapter parsing | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_handler_returns_401_when_unconfigured |
| `qa-telegram:A2`<br>An unpaired Telegram DM receives one throttled setup hint without starting a turn | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_dispatch::{unpaired_hints_are_throttled_per_chat, hint_throttle_prunes_entries_older_than_window, classify_inbound_text_covers_admission_rows} |
| `qa-telegram:A3`<br>A bare Telegram start command never mints a pairing code | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_dispatch::classify_inbound_text_covers_admission_rows (bare `/start` → StartWithoutPayload → static hint; codes only originate web-side, pinned by telegram_pairing::issue_* tests) |
| `qa-telegram:A4`<br>DRAFT — qa-telegram:A4 | **draft** | DRAFT parent — carried by qa-telegram:A4:01 |
| `qa-telegram:A4:01:cross-state-header-rejection-matrix`<br>Telegram rejects missing and forged verification headers in every setup state | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_handler_returns_401_on_{missing_secret_header,wrong_secret_header,auth_failure} + _returns_401_when_unconfigured; live-route forged-secret probe in `cargo test --test reborn_integration_telegram_journey` (whole-journey scenario) |
| `qa-telegram:A5`<br>Telegram drops group, channel, edit, and bot-authored updates without side effects | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_dispatch::non_actionable_updates_are_acked_silently + `cargo test -p ironclaw_telegram_v2_adapter` payload::{channel_post_is_noop, edited_message_is_noop, group_ambient_message_is_noop, message_without_from_classifies_as_noop_not_error} |
| `qa-telegram:B1`<br>DRAFT — qa-telegram:B1 | **draft** | DRAFT parent — carried by B1:01/B1:02/B1:04 |
| `qa-telegram:B1:01:telegram-admin-setup-save`<br>An administrator saves Telegram host configuration through Admin > Channels | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey seam 1 (PUT through the real protected route; getMe+setWebhook captured at the network boundary) + `cargo test -p ironclaw_telegram_extension` telegram_setup::save_happy_path_validates_registers_and_persists + telegram_channel_routes::handler_tests |
| `qa-telegram:B1:02:telegram-status-redaction`<br>Telegram channel status reveals readiness but never host secret values | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_channel_routes::handler_tests::get_setup_returns_redacted_status_without_secret_values + `cargo test --test reborn_integration_telegram_journey` (redacted GET assert) |
| `qa-telegram:B1:04:first-paired-inbound`<br>The first paired Telegram DM enters the canonical runner and receives one reply | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey seam 4 (paired DM → canonical runner → one rendered reply) |
| `qa-telegram:B2`<br>Invalid Telegram bot credentials are rejected without changing channel state | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_setup::invalid_token_persists_nothing |
| `qa-telegram:B3`<br>Failed Telegram webhook replacement rolls back to the prior ready setup | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_setup::{set_webhook_failure_persists_nothing, rollback_restores_previous_record_and_previous_webhook_registration, failed_record_persist_restores_previous_webhook_for_same_bot} |
| `qa-telegram:B4`<br>Telegram setup refuses to save without an HTTPS public callback base | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_setup::missing_public_base_url_fails_before_any_bot_api_call_after_validation + telegram_channel_routes::handler_tests::save_setup_rejects_injection_shaped_webhook_url |
| `qa-telegram:B5`<br>Rotating a Telegram token for the same bot cuts over atomically | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_setup::rotation_bumps_revision_and_keeps_installation_identity + telegram_serve::telegram_updates_dispatch_after_first_configure_without_rebuild |
| `qa-telegram:B6`<br>Switching Telegram to a different bot invalidates old member pairings | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_bot_swap_rekeys_workflow_and_rejects_old_secret + telegram_host_state::installation_scope_matching_is_exact_never_prefix_overlap (pairings are installation-scoped; a new bot id orphans them) |
| `qa-telegram:B7`<br>DRAFT — qa-telegram:B7 | **draft** | DRAFT parent — carried by B7:01 |
| `qa-telegram:B7:01:telegram-setup-api-authorization`<br>Telegram setup APIs distinguish member denial from masked cross-tenant targets | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_channel_routes::handler_tests::{setup_routes_mask_cross_tenant_probes_as_not_found, setup_routes_forbid_same_tenant_member_but_pairing_is_self_service} |
| `qa-telegram:C1`<br>A paired Telegram DM routes to its member-owned thread | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey seam 4 + telegram_two_users_stay_isolated_across_pairing_reply_and_unpair (thread ownership per bound user) |
| `qa-telegram:C3`<br>Telegram splits responses over 4096 characters into ordered lossless messages | **covered** | Implemented 2026-07-17 (was gap-product): `cargo test -p ironclaw_telegram_v2_adapter` render::{final_reply_over_4096_units_splits_into_ordered_lossless_chunks, chunk_boundaries_never_split_a_surrogate_pair} + adapter::{render_outbound_sends_chunks_sequentially_and_records_one_delivered, render_outbound_records_retryable_when_a_middle_chunk_fails}. Chunking is UTF-16-unit based (Telegram length semantics); mid-sequence failure stops and records one honest FailedRetryable. |
| `qa-telegram:C4`<br>Telegram sends deterministic plain text without parse_mode | **covered** | `cargo test -p ironclaw_telegram_v2_adapter` render::final_reply_renders_with_topic_and_reply_target (body carries chat_id/text only — no parse_mode key) + `cargo test --test reborn_integration_telegram_journey` rendered-reply asserts |
| `qa-telegram:C5`<br>Telegram routes attachment captions but refuses captionless media without a model turn | **partial** | `cargo test -p ironclaw_telegram_v2_adapter` payload::private_chat_with_photo_emits_attachment_descriptor_no_bytes (caption routing) + `cargo test -p ironclaw_telegram_extension` telegram_dispatch::textless_message_follows_pairing_split. The 'model-visible attachment note for captionless media' half is model-behavior — Recorded-Model tier. |
| `qa-telegram:C6`<br>A paired Telegram start command is a static no-op | **covered** | Implemented 2026-07-17 (was a shipped-vs-row divergence — the hint was pairedness-agnostic): `cargo test -p ironclaw_telegram_extension` telegram_dispatch::{paired_start_without_payload_is_a_silent_no_op, start_without_payload_acks_silently_when_lookup_is_down}. |
| `qa-telegram:C7`<br>Telegram preserves valid Unicode and escaped controls through the conversation path | **partial** | `cargo test -p ironclaw_telegram_v2_adapter` payload::slice_tests::multibyte_slice_respects_utf16_offsets (+ command-argument control-char rejection via shared validation). A full unicode round-trip through the composed stack is not separately pinned; the journey drives ASCII. |
| `qa-telegram:C9`<br>DRAFT — qa-telegram:C9 | **draft** | DRAFT placeholder — no executable sub-rows yet |
| `qa-telegram:D1`<br>DRAFT — qa-telegram:D1 | **draft** | DRAFT parent — carried by D1:01/D1:02 |
| `qa-telegram:D1:01:telegram-thread-isolation`<br>Telegram messages create threads owned by their bound users | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_two_users_stay_isolated_across_pairing_reply_and_unpair |
| `qa-telegram:D1:02:telegram-reply-isolation`<br>Telegram replies return only to the originating bound user | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_two_users_stay_isolated_across_pairing_reply_and_unpair (per-chat replies + no-cross-bleed matrix) |
| `qa-telegram:D2`<br>An unknown Telegram user receives only the static pairing hint | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_dispatch::{unpaired_hints_are_throttled_per_chat} + `cargo test --test reborn_integration_telegram_journey` multi-user scenario R8 leg (post-unpair DM → hint, no turn) |
| `qa-telegram:D4`<br>Unpairing User A leaves User B’s Telegram binding unchanged | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_two_users_stay_isolated_across_pairing_reply_and_unpair (A unpairs; B's binding, thread, and delivery untouched) |
| `qa-telegram:F1`<br>A retried Telegram update produces exactly one turn and one reply | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_duplicate_updates_and_send_failures_stay_honest (redelivered update_id → exactly one turn/reply) + `cargo test -p ironclaw_telegram_v2_adapter` idempotency contract |
| `qa-telegram:F2`<br>A Telegram send outage records Failed instead of optimistic delivery | **covered** | `cargo test -p ironclaw_telegram_v2_adapter` render_outbound_records_{retryable_on_telegram_5xx, unauthorized_on_telegram_401, permanent_on_telegram_400, delivered_on_2xx} (the honesty mapping) + `cargo test --test reborn_integration_telegram_journey` outage leg (no crash, no optimistic delivery) |
| `qa-telegram:F3`<br>DRAFT — qa-telegram:F3 | **draft** | DRAFT parent — carried by F3:01/F3:02 |
| `qa-telegram:F3:01:telegram-send-403`<br>A Telegram 403 marks the delivery failed and does not retry storm | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_duplicate_updates_and_send_failures_stay_honest (403 → exactly one Bot API attempt, no retry storm) + `cargo test -p ironclaw_telegram_v2_adapter` unauthorized mapping |
| `qa-telegram:F3:02:telegram-send-recovery`<br>The next Telegram send succeeds after a prior blocked-recipient failure clears | **covered** | `cargo test --test reborn_integration_telegram_journey` telegram_duplicate_updates_and_send_failures_stay_honest (next turn's reply delivers after the failure clears) |
| `qa-telegram:F4`<br>Telegram honors one retry_after delay and stops after the bounded retry | **covered** | Implemented 2026-07-17 (was gap-product): `cargo test -p ironclaw_telegram_extension` telegram_egress::{send_retries_once_on_429_honoring_retry_after, send_returns_second_429_honestly_without_a_third_attempt, send_does_not_retry_when_retry_after_exceeds_the_cap, send_does_not_retry_a_429_without_retry_after} (cap 5s; paused-clock delay assertions). |
| `qa-telegram:F6`<br>Telegram verifier swaps generations atomically below update deduplication | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_bot_swap_rekeys_workflow_and_rejects_old_secret (+ shared idempotency ledger keyed by host scope, not revision — see telegram_host_beta.rs ledger comment) |
| `qa-telegram:F7`<br>Telegram ignores callback, membership, inline, and channel-only updates | **covered** | `cargo test -p ironclaw_telegram_v2_adapter` payload noop tests + `cargo test -p ironclaw_telegram_extension` telegram_dispatch::non_actionable_updates_are_acked_silently |
| `qa-telegram:F8`<br>A revoked Telegram bot token fails honestly until an administrator saves a valid replacement | **covered** | `cargo test -p ironclaw_telegram_v2_adapter` render_outbound_records_unauthorized_on_telegram_401 (honest failure while revoked) + `cargo test -p ironclaw_telegram_extension` telegram_setup::{rotation_bumps_revision_and_keeps_installation_identity, save_happy_path…} (recovery by re-save) |
| `qa-telegram:I2`<br>DRAFT — qa-telegram:I2 | **draft** | DRAFT parent — carried by I2:02 |
| `qa-telegram:I2:02:telegram-blocked-run-gate`<br>Telegram pairing completion resumes its blocked chat run exactly once | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey (park → consume → resume exactly once) + `cargo test -p ironclaw_telegram_extension` telegram_pairing repair test (at-least-once dispatch is idempotent on completion) |
| `qa-telegram:I4`<br>DRAFT — qa-telegram:I4 | **draft** | DRAFT parent — carried by I4:03 |
| `qa-telegram:I4:03:admin-required-blocked-output`<br>A Telegram run blocked before admin setup terminates with member-safe guidance | **covered** | composition suite (`--lib`) extension_lifecycle telegram gate tests (unfilled TelegramPairedStatusSlot → fail-closed 'not enabled on this deployment'; pairing-status outage → transient) + `cargo test -p ironclaw_telegram_extension` telegram_pairing::issue_fails_closed_when_unconfigured ('administrator must configure the Telegram bot first') |
| `qa-telegram:I5`<br>DRAFT — qa-telegram:I5 | **draft** | DRAFT placeholder — no executable sub-rows yet |
| `qa-telegram:P1`<br>DRAFT — qa-telegram:P1 | **draft** | DRAFT parent — carried by P1:01/P1:02/P1:03 |
| `qa-telegram:P10`<br>DRAFT — qa-telegram:P10 | **draft** | DRAFT placeholder — no executable sub-rows yet |
| `qa-telegram:P12`<br>Disconnecting Telegram revokes the member binding and returns to pairing-required | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::{unpair_removes_binding_target_and_pending_code, unpair_after_admin_cleared_setup_still_removes_the_binding} + telegram_channel_routes::handler_tests::disconnect_pairing_unpairs_only_the_caller + `cargo test --test reborn_integration_telegram_journey` multi-user scenario |
| `qa-telegram:P13`<br>Invalid Telegram pairing guesses are uniformly rejected and rate-limited | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_dispatch::invalid_code_replies_are_throttled_per_chat_without_gating_valid_consume (landed with fix 937ecebda) |
| `qa-telegram:P14`<br>DRAFT — qa-telegram:P14 | **draft** | DRAFT placeholder — no executable sub-rows yet |
| `qa-telegram:P1:01:pairing-code-consume`<br>A valid Telegram deep-link code binds only the existing IronClaw member | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::consume_happy_path_binds_targets_and_dispatches + `cargo test --test reborn_integration_telegram_journey` whole-journey seam 3 |
| `qa-telegram:P1:02:bind-never-mint-identity`<br>Telegram pairing binds an existing user and never mints an identity | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::consume_* (binding an EXISTING user id) + telegram_actor_identity::resolver_gates_on_adapter_id_and_actor_kind; unknown senders never reach the workflow (`cargo test -p ironclaw_telegram_extension` telegram_dispatch admission tests) |
| `qa-telegram:P1:03:pairing-continuation-fanout`<br>One Telegram pairing completion resumes every matching blocked run exactly once | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::{consume_happy_path_binds_targets_and_dispatches, resend_after_failed_continuation_dispatch_repairs_completion} + `cargo test --test reborn_integration_telegram_journey` whole-journey (BlockedAuth → Completed via the real fanout) |
| `qa-telegram:P2`<br>DRAFT — qa-telegram:P2 | **draft** | DRAFT parent — carried by P2:01 |
| `qa-telegram:P2:01:typed-code-exact-match`<br>A bare pairing code matches case-insensitively but rejects surrounding text | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_dispatch::{bare_lowercase_code_pairs, classify_inbound_text_covers_admission_rows} (case-insensitive exact match; surrounded text classifies Ordinary) |
| `qa-telegram:P3`<br>DRAFT — qa-telegram:P3 | **draft** | DRAFT parent — carried by P3:01 |
| `qa-telegram:P3:01:pairing-code-ttl`<br>An expired Telegram pairing code cannot create a binding | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_host_state::claim_pairing_refuses_expired_codes + telegram_pairing::issue_mints_code_with_deep_link_and_ttl |
| `qa-telegram:P4`<br>A consumed Telegram pairing code cannot be reused | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_host_state::claim_pairing_is_single_consumer_and_keeps_the_receipt + telegram_pairing::concurrent_consume_of_one_code_binds_exactly_one_winner |
| `qa-telegram:P5`<br>Explicit Telegram pairing renewal rotates the code while passive reopen does not | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::reissue_rotates_and_kills_the_old_code (explicit renewal rotates; passive status reads don't — pinned by issue/status contract tests) |
| `qa-telegram:P6`<br>A Telegram identity already paired to another user cannot be reassigned by code | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::telegram_account_bound_to_other_user_is_refused + `cargo test --test reborn_integration_telegram_journey` multi-user scenario (refusal copy over the live webhook; A's binding survives) |
| `qa-telegram:P7`<br>Re-pairing the same Telegram identity to the same user is idempotent | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::same_user_re_pair_is_idempotent |
| `qa-telegram:P8`<br>DRAFT — qa-telegram:P8 | **draft** | DRAFT parent — carried by P8:01 |
| `qa-telegram:P8:01:provider-keyed-resume-fanout`<br>Page-initiated Telegram pairing resumes a chat-blocked run by provider key | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey (page-minted code resumes the chat-parked run; gates are provider-keyed, codes rotate freely) + composition suite (`--lib`) blocked-auth fanout suites |
| `qa-telegram:P9`<br>One Telegram binding resumes all matching blocked threads and no unrelated gate | **covered** | composition suite (`--lib`) BlockedAuthResumeFanout suites (provider-keyed, resumes every matching run, unrelated gates untouched) + `cargo test --test reborn_integration_telegram_journey` resume assert |
| `qa-telegram:R1`<br>DRAFT — qa-telegram:R1 | **draft** | DRAFT parent — carried by R1:01…R1:06 |
| `qa-telegram:R1:01:telegram-configure`<br>The clean lifecycle walk establishes Telegram through Admin > Channels | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey seam 1 |
| `qa-telegram:R1:02:telegram-pair`<br>A freshly installed Telegram channel accepts one member pairing | **covered** | `cargo test --test reborn_integration_telegram_journey` whole-journey seam 3 |
| `qa-telegram:R1:03:telegram-remove`<br>Telegram removal waits for one trusted cleanup receipt before deleting the installation | **covered** | composition suite (`--lib`) extension_removal_cleanup::{telegram_removal_cleanup_disconnects_the_removing_user, registry_fails_closed_for_unknown_required_adapter, registry_dispatches_requirements_in_deterministic_order} (removal waits on the trusted cleanup receipt) |
| `qa-telegram:R1:04:telegram-stale-target`<br>A removed Telegram binding rejects its stale delivery target without contacting the provider | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_outbound_targets::{list_is_empty_when_caller_is_unpaired, list_is_empty_when_unconfigured} + composition suite (`--lib`) channel_delivery driver no-target tests (stale target → honest skip, no provider contact) |
| `qa-telegram:R1:05:telegram-reinstall`<br>Telegram reinstall creates a fresh member pairing under the existing admin bot | **partial** | Re-pair-after-unpair is pinned crate-side (`cargo test -p ironclaw_telegram_extension` telegram_pairing::{unpair_removes…, issue_mints…} — fresh code post-unpair) but a composed reinstall walk (remove extension → reinstall → fresh pairing) is not driven end to end; fold candidates for the group_extensions state-machine suite. |
| `qa-telegram:R1:06:telegram-repair`<br>A reinstalled Telegram channel requires and accepts a fresh member pairing | **partial** | Same composed-walk caveat as R1:05; the fresh-pairing requirement itself is pinned by the lifecycle gate (unpaired ⇒ pairing requirement) + crate pairing tests. |
| `qa-telegram:R2`<br>Removing Telegram unpairs only the requesting member | **covered** | composition suite (`--lib`) extension_removal_cleanup::telegram_removal_cleanup_disconnects_the_removing_user + `cargo test --test reborn_integration_telegram_journey` multi-user scenario (self-service disconnect leg) |
| `qa-telegram:R3`<br>Clearing Telegram in Admin > Channels disables ingress and deletes only deployment secrets | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_setup::clear_deletes_webhook_and_record + telegram_serve::telegram_updates_handler_returns_401_when_unconfigured (ingress fails closed after clear; pairings retained by design — see telegram_pairing::unpair_after_admin_cleared_setup_still_removes_the_binding for the retained-store shape) |
| `qa-telegram:R5`<br>Removing Telegram invalidates the member's pending pairing code | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_pairing::unpair_removes_binding_target_and_pending_code |
| `qa-telegram:R8`<br>A Telegram DM after member removal returns a connection hint without starting a turn | **covered** | `cargo test --test reborn_integration_telegram_journey` multi-user scenario (post-unpair DM → static hint, no turn) |
| `qa-telegram:S1`<br>A configured Telegram webhook rejects a missing secret header before parsing | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_handler_returns_401_on_missing_secret_header |
| `qa-telegram:S2`<br>DRAFT — qa-telegram:S2 | **draft** | DRAFT placeholder — no executable sub-rows yet |
| `qa-telegram:S3`<br>Telegram rejects duplicate verification headers without first-or-last wins behavior | **covered** | `cargo test -p ironclaw_wasm_product_adapters` auth_verifier duplicate-header rejection (landed with fix 04df727ff) |
| `qa-telegram:S4`<br>Telegram rejects webhook bodies over the configured limit before JSON parsing | **covered** | composition tests/webui_v2_serve.rs telegram_public_mount_enforces_descriptor_body_limit_and_404s_path_probes (>1 MiB → 413 through the COMPOSED listener before verification; in-budget control reaches the fail-closed verifier). Descriptor value pinned by telegram_serve::telegram_updates_route_descriptor_matches_manifest_projection. |
| `qa-telegram:S5`<br>Malformed Telegram JSON fails safely without a turn or unsanitized log | **divergence** | SHIPPED ≠ ROW: verified-but-unparseable JSON is deliberately acked 200 with no turn/reply/body-echo (anti-redelivery — Telegram retries non-2xx forever); the row drafts a 4xx. Pinned by `cargo test -p ironclaw_telegram_extension` telegram_serve::telegram_updates_handler_acks_malformed_json_without_turn_or_reply. Unverified malformed bodies are 401 before parsing. |
| `qa-telegram:S6`<br>Telegram path probes fail before extension resolution or store access | **covered** | composition tests/webui_v2_serve.rs telegram_public_mount_enforces_descriptor_body_limit_and_404s_path_probes (/webhooks/extensions/telegram/bogus → 404 at the router; an unmounted path never reaches the installation resolver). |
| `qa-telegram:S7`<br>DRAFT — qa-telegram:S7 | **draft** | DRAFT parent — carried by S7:01/S7:02 |
| `qa-telegram:S7:01:setup-api-secret-redaction`<br>Telegram setup status returns readiness without secret values | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_channel_routes::handler_tests::get_setup_returns_redacted_status_without_secret_values |
| `qa-telegram:S7:02:trace-secret-redaction`<br>Telegram egress failures redact credentials from logs and traces | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_egress::{telegram_bot_api_url_carries_placeholder_and_never_raw_token, rejects_control_chars, rejects_unknown_handle} + telegram_bot_api::{parse_garbage_is_invalid_response_without_body_echo, parse_not_ok_envelope_maps_to_stable_rejection_category} (provider text never rides errors) |
| `qa-telegram:V1`<br>A routine result is delivered once to the paired Telegram target | **covered** | `cargo test -p ironclaw_reborn_composition telegram_target_is_enumerated_resolved_and_delivered_through_generic_wiring` drives the real bundled manifest and Telegram adapter through generic DM-target enumeration, caller-scoped resolution, the triggered-delivery hook, and Bot API egress without a fabricated resolver ref. |
| `qa-telegram:V2`<br>Telegram receives heartbeat findings and stays silent when nothing is found | **not-automated** | Heartbeat findings delivery is runtime/live behavior (heartbeat scheduler + real workspace); the deterministic tier covers the delivery seam (V1 parts) — heartbeat itself is the Runtime Integration/Live Canary tier. |
| `qa-telegram:V3`<br>DRAFT — qa-telegram:V3 | **draft** | DRAFT placeholder — no executable sub-rows yet |
| `qa-telegram:V4`<br>A proactive Telegram result uses only its owner's stored chat target | **covered** | `cargo test -p ironclaw_telegram_extension` telegram_outbound_targets::{paired_caller_gets_dm_entry_with_canonical_binding_ref, list_is_empty_for_cross_tenant_caller, resolve_outbound_delivery_target_default_impl_matches_own_id_only} (+ owner-scope defense-in-depth check in the provider) |
| `qa-telegram:V5`<br>Missing Telegram targets fail honestly or use the configured Web fallback | **covered** | composition suite (`--lib`) channel_delivery::driver_no_preference_records_no_default_configured_without_egress (+ Web fallback rows live with the outbound-preferences suites) |

## Unit / Contract (3 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `coverage:installable-catalog:exact-package-set`<br>The built-in catalog exposes exactly the twelve supported installable packages | **covered** | Adapter/unit contract rows — `cargo test -p ironclaw_telegram_v2_adapter --lib` per-module tests. |
| `qa-telegram:P14:01:pairing-qr-payload`<br>The Telegram pairing QR encodes only the current bot deep link | **covered** | Adapter/unit contract rows — `cargo test -p ironclaw_telegram_v2_adapter --lib` per-module tests. |
| `qa-telegram:S2:01:constant-time-header-verifier`<br>The Telegram shared-secret verifier uses constant-time equality | **covered** | Adapter/unit contract rows — `cargo test -p ironclaw_telegram_v2_adapter --lib` per-module tests. |

## Static / Architecture (4 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `qa-telegram:I5:01:telegram-registry-shape`<br>The Registry exposes one installable Telegram channel package with zero tools | **covered** | `cargo test -p ironclaw_architecture --test telegram_extension_gates` (retired-taxonomy zero + no v1 pairing-route literals) and the layer/boundary suites. |
| `qa-telegram:I5:02:telegram-forbidden-identifiers`<br>Legacy Telegram identifiers and the removed member Channels route stay absent | **covered** | `cargo test -p ironclaw_architecture --test telegram_extension_gates` (retired-taxonomy zero + no v1 pairing-route literals) and the layer/boundary suites. |
| `qa-telegram:S7:03:webhook-url-secret-substitution`<br>Telegram webhook registration uses a generated secret handle without exposing it to product layers | **covered** | `cargo test -p ironclaw_architecture --test telegram_extension_gates` (retired-taxonomy zero + no v1 pairing-route literals) and the layer/boundary suites. |
| `qa-telegram:V3:01:no-telegram-tool-registry`<br>Telegram contributes zero callable tools to every capability registry | **covered** | `cargo test -p ironclaw_architecture --test telegram_extension_gates` (retired-taxonomy zero + no v1 pairing-route literals) and the layer/boundary suites. |

## Runtime Integration (9 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `qa-telegram:C8`<br>A rapid Telegram DM burst stays ordered and reports every rate-limited message | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:D3`<br>A revoked Telegram binding is rejected in flight and on the next message | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:D6`<br>Concurrent Telegram pairings create two independent bindings | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:F5`<br>A Telegram adapter panic is isolated and the next update still succeeds | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:P10:01:pairing-gate-restart`<br>A Telegram pairing gate survives thread switch, reload, and server restart | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:P3:02:expired-gate-durability`<br>An expired Telegram pairing code can renew after restart without losing its gate | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:R4`<br>Telegram setup and member bindings reopen after restart without provider re-registration | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:R6`<br>A Telegram update paused before epoch validation cannot dispatch after member removal | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |
| `qa-telegram:R7`<br>Resaving the active Telegram bot during traffic preserves accepted updates and one webhook generation | **not-automated** | Needs the runtime lane (real scheduler clock/Docker/OS service shape) — cargo test --features integration tier, not the in-process harness. |

## Recorded-Model Hermetic (10 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `qa-remove-reconfigure:RC-10:06:chat-channel-configure`<br>Member chat cannot configure a deployment channel and directs administrators to Admin > Channels | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-routines:S11:01:telegram-routine-create-choice`<br>A Telegram scheduling request proposes trigger_create for the paired owner | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:C10`<br>A Telegram-initiated built-in action uses the same approval and audit pipeline as WebChat | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:C2`<br>Follow-up Telegram DMs preserve ordered conversation context | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:D1:03:memory-context-isolation`<br>Telegram conversations inject only the paired user’s memory context | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:D5`<br>A model request cannot route Telegram into another user’s thread | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:I2:01:telegram-install-tool-choice`<br>Chat selects Telegram and parks one run on the member pairing gate | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:I4:02:unconfigured-chat-choice`<br>Chat reports administrator setup required without minting a Telegram pairing gate | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:I6`<br>Chat installs Telegram and reaches derived active state in one pass when the member is already paired | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |
| `qa-telegram:V3:02:unsupported-tool-response`<br>Requests to read or send Telegram messages receive an honest unsupported response | **not-automated** | Pins real model tool-choice/wording — needs recorded traces (tests/fixtures/llm_traces + scrub gate), not scripted replies. |

## Browser E2E (27 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `qa-install:A-4a:01:telegram-page-install-entry`<br>Telegram page install exposes member pairing without tenant bot secrets | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-remove-reconfigure:RC-10:03:browser-channel-configure`<br>Only an administrator can edit Telegram deployment configuration in Admin > Channels | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-remove-reconfigure:RC-2:01:telegram-admin-token-save`<br>The Admin > Channels form masks Telegram secrets and omits blank existing values from save | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-remove-reconfigure:RM-H8:01:telegram-user-removal`<br>A member confirms Telegram removal without seeing or changing admin bot configuration | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:B1:03:telegram-setup-browser-state`<br>Telegram admin save updates browser status and serves a pre-paired DM without restart | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:B7:02:telegram-setup-ui-visibility`<br>Only operators see Telegram channel configuration in the Admin UI | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I1`<br>The Extensions page installs Telegram and completes member pairing | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I2:03:telegram-pairing-card-render`<br>A blocked Telegram run renders pairing chrome outside model-visible content | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I3`<br>Chat and Extensions pairing panels share one durable Telegram contract | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I4:01:unconfigured-page-install`<br>Telegram page install projects setup needed before tenant bot setup | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I5:03:telegram-catalog-fields`<br>Telegram Registry fields expose member install and connect state without host configuration | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I5:04:telegram-catalog-network-calls`<br>Telegram Registry install and connect use only current lifecycle endpoints | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:I7`<br>Telegram pairing completion updates open page and chat projections live | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P10:02:pairing-gate-thread-switch`<br>Switching WebChat threads preserves the Telegram pairing gate in its owner thread | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P10:03:pairing-gate-page-reload`<br>Reloading WebChat rehydrates the same durable Telegram pairing gate | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P11`<br>Dismissing and reopening a Telegram pairing card does not mutate its gate | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P14:02:pairing-qr-renewal`<br>Renewal replaces the Telegram QR and makes a stale QR fail closed | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P14:03:cross-device-live-status`<br>Phone-completed Telegram pairing updates the desktop panel in real time | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P1:04:pairing-live-status-browser`<br>Telegram pairing updates the open member panel and blocked chat without a refresh | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P2:02:pairing-code-copy-affordance`<br>The Telegram pairing panel exposes a keyboard-copyable code and searchable bot username | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P3:03:pairing-countdown-render`<br>The Telegram pairing countdown reaches expiry without showing a usable stale code | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P3:04:cross-surface-code-renewal`<br>Renewing a Telegram pairing code replaces it on every open member surface | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:P8:02:page-pairing-chat-update`<br>Pairing from the Extensions page updates the same member's blocked chat | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-telegram:S7:04:browser-secret-redaction`<br>Telegram member and admin pages never render stored secret values | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-webui:EX4`<br>Install and setup render the correct derived active or setup-needed outcome for ready, auth-gated, and channel packages | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-webui:LV7`<br>A cross-tab channel connection wakes and resumes the exact waiting thread once | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |
| `qa-webui:SP3`<br>Connecting Telegram on the Extensions page clears a same-tab chat gate | **not-automated** | Browser-visible flow — belongs to tests/e2e Playwright (reborn_v2_* fixtures); not automatable in the hermetic Rust tier. |

## Live Canary (11 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `derived:telegram:functional-smoke`<br>DRAFT — derived:telegram:functional-smoke | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `derived:telegram:functional-smoke:01:pairing-readiness`<br>A disposable Telegram identity reaches paired-ready state | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `derived:telegram:functional-smoke:02:dm-round-trip`<br>A paired Telegram member receives one deterministic DM reply | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `derived:telegram:functional-smoke:03:restart-recovery`<br>Telegram delivery resumes through the same binding after restart | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `derived:telegram:functional-smoke:04:cross-member-isolation`<br>An unknown Telegram user cannot enter User A’s thread | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `qa-install:C6:13:live-telegram-readiness`<br>A serialized Telegram install exposes member pairing without reopening shared bot setup | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `qa-telegram:B1:05:live-telegram-api-compatibility`<br>A real configured Telegram bot accepts one paired DM and returns the canary reply | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `qa-telegram:F3:03:live-block-unblock`<br>A real Telegram block fails delivery and unblocking restores only the next send | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `qa-telegram:P1:05:live-telegram-deep-link`<br>A real Telegram deep link opens the configured bot and consumes its payload once | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `qa-telegram:P2:03:existing-chat-client-behavior`<br>A typed pairing code works in an existing real Telegram chat | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |
| `qa-telegram:R1:07:live-telegram-lifecycle-smoke`<br>A real Telegram channel works after member removal and fresh re-pairing | **not-automated** | Requires live Telegram (BotFather bot, real api.telegram.org) — owner-held credentials; deterministic tiers cover the seams, the live lane covers drift. |

## Manual QA (1 rows)

| Row | Status | Evidence / reason |
|---|---|---|
| `qa-telegram:P14:04:phone-camera-launch`<br>A phone camera opens the installed Telegram client at the intended bot | **not-automated** | Human-judgment row (manual QA journey). |

## Tally

- **covered**: 72
- **divergence**: 1
- **draft**: 19
- **gap-product**: 0
- **needs-test**: 2
- **not-automated**: 59
- **partial**: 7

## Owner adjudications requested

1. **qa-telegram:S5 (malformed JSON)** — shipped: verified-but-unparseable
   bodies are acked 200 (anti-redelivery) with no turn/reply/body-echo; the
   row drafts a 4xx. The shipped shape is pinned by test; flip either the row
   or the classification.

(Resolved 2026-07-17: C3 chunking, F4 429-retry, and C6 pairedness-aware
`/start` were implemented with red-first tests — their rows above are
**covered**.)

## Live-session findings (2026-07-17, not drafted as catalog rows)

Found by live QA on the local stack; catalog amendment candidates. Each
describes shared channel-host machinery assuming a Slack-side capability the
Telegram side lacks — the sweep dimension the original 16-scenario suite
missed (it pinned what Telegram does, not what the shared layer advertises).

| Finding | Status | Evidence / fix |
|---|---|---|
| A `BlockedAuth` run with a link-shaped challenge DMs the authorization URL (was: adapter recorded the AuthPrompt `Deferred` → thinking-message deleted, then silence) | **fixed + covered** | `cargo test -p ironclaw_telegram_v2_adapter` adapter::{render_outbound_auth_prompt_sends_link_message_and_records_delivered, render_outbound_gate_prompt_sends_webapp_redirect_and_records_delivered} + `cargo test --test reborn_integration_telegram_journey` telegram_dm_gated_install_posts_oauth_authorization_link_not_silence (proven red pre-fix) |
| Busy-on-auth hint advertises ``auth deny <gate_ref>`` in-chat, but Telegram inbound had no interaction-resolution parsing — the reply bounced off the busy thread with the same hint (phantom affordance loop) | **fixed + covered** | Shared grammar `ironclaw_product_adapters::interaction_commands` (Slack delegates; Telegram parses in `build_payload`); drift guards round-trip advertised copy through the parser (driver + adapter tiers); journey pin `telegram_dm_auth_deny_command_cancels_gate_and_frees_the_thread` (hint → command → cancel → "Authentication canceled." → thread frees; proven red pre-fix) |
| Telegram unpair/removal did not clean the conversation-actor pairing: after re-pair the chat re-attached to the old thread and its parked runs | **fixed + covered** | `telegram_pairing.rs::unpair` now unpairs the conversation actor with the removed binding's epoch (Slack disconnect parity; `unbind` returns `RemovedTelegramBinding{provider_user_id, epoch}`); crate pin in `unpair_removes_binding_target_and_pending_code`; journey pin `telegram_unpair_then_repair_starts_fresh_thread_not_the_old_blocked_one` (proven red pre-fix incl. the epochless no-op) |
