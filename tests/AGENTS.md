# Scenario Test Coverage Map

This document is the inventory of **every scenario test in the repo**, written in
plain English and mapped back to the exact file (and where useful, the exact test
name) that proves it. It answers two questions:

1. *Is this user-visible behavior already covered, and by what?*
2. *What is not covered yet?*

## MAINTENANCE RULE (non-negotiable)

**When you add, rename, delete, or materially re-scope a scenario test, update this
document in the SAME commit.**

- New `tests/integration/group_*/scenario_*.rs` → add a row to §3.
- New `tests/integration/<name>.rs` bin → add a row to §4.
- New `tests/integration/auth/<name>.rs` bin → add a row to §4.
- New `tests/*.rs` bin → add a row to §5.
- New `tests/e2e/scenarios/test_*.py` → add a row to §6.
- Closing a gap listed in §7 → delete that gap row and add the coverage row.
- Deleting/renaming a test → fix or remove the row that cites it; a row citing a
  file or test name that no longer exists is a broken document.

Rows describe **what a user can do or observe**, not what a function returns. If you
cannot write the row in one plain-English sentence, the test is probably asserting an
internal rather than a behavior — see `.claude/rules/testing.md`.

The counts in each section header are checked-in facts, not estimates. Update
them in the same commit; re-derive any of them with:

- Groups (the count of `group_*/` directories): `find tests/integration -maxdepth
  1 -type d -name 'group_*' | wc -l`. Scenarios inside one named group (e.g.
  `group_approvals`): `find tests/integration/group_approvals -maxdepth 1 -name
  'scenario_*.rs' | wc -l`.
- Flat integration bins: `ls tests/integration/*.rs tests/integration/auth/*.rs
  | wc -l`.
- Top-level Rust bins: `ls tests/*.rs | wc -l`.
- E2E files: `ls tests/e2e/scenarios/test_*.py | wc -l`.
- E2E test *functions* (the §2 "889" figure — every `test_*` function or
  method, top-level or in a class, across all 103 files; this is an exhaustive
  syntactic count, not filtered by active/legacy status):
  ```
  python3 -c "
  import ast, glob
  n = 0
  for f in glob.glob('tests/e2e/scenarios/test_*.py'):
      for node in ast.walk(ast.parse(open(f).read())):
          if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith('test_'):
              n += 1
  print(n)
  "
  ```
- E2E collected pytest items (the §6 "1180 top-level tests" figure — pytest's
  own collection count, which differs from the syntactic function count above
  when a function is parametrized, skipped at collection, or a class groups
  several `test_*` methods under one node): `cd tests/e2e && python3 -m pytest
  scenarios --collect-only -q | tail -1` (requires the E2E dependencies from
  `tests/e2e/pyproject.toml`, e.g. via `uv run` or the project's E2E venv).
  Neither figure filters by whether a scenario is currently
  Reborn-executable — see the §6 preamble.

---

## 1. Where scenarios live

This is a file-location index into the inventory below, not a second
test-tier taxonomy — **tier selection for new work follows
`.claude/rules/testing.md`.** The four rows below correspond to that rule's
tiers 2 (in-process Reborn integration), 5 (recorded model behavior), and 6
(browser/E2E); tiers 1, 3, 4, and 7 (unit/contract, architecture,
backend/runtime integration, live canary) have no scenario rows here.

| Rows in this doc | Location | What is real | What is faked | Cost |
|---|---|---|---|---|
| **Group scenarios** (§3) | `tests/integration/group_*/scenario_*.rs` | whole Reborn turn, shared runtime + shared stores across several threads | the model, at the vendor-SDK seam | fast, offline, no setup |
| **Flat integration** (§4) | `tests/integration/*.rs`, `tests/integration/auth/*.rs` | whole Reborn turn, one thread; one dedicated sandbox test also uses a real local Docker worker | the model | usually fast/offline; sandbox worker test runs in its Docker CI lane |
| **Binary / parity / QA-trace** (§5) | `tests/*.rs` | composed runtime or shipping binary; recorded real-LLM traces | model responses replayed from committed traces | medium |
| **Python E2E** (§6) | `tests/e2e/scenarios/test_*.py` | current Reborn scenarios use real `ironclaw serve`, HTTP, and Playwright; retained legacy-fixture scenarios are pending migration | LLM (mock or recorded), external SaaS (Emulate/fakes) | slow |

Authoring guides: `tests/integration/AGENTS.md` (Rust harness, group mechanics,
assertions) and `tests/e2e/AGENTS.md` (pytest fixtures, Playwright, mock LLM).

## 2. Coverage at a glance

| Area | Group | Flat int | Binary/QA | Python E2E |
|---|---|---|---|---|
| Approvals & permission gates | 10 | ✓ | ✓ | ✓ |
| Auth / credentials / OAuth | 7 | 7 | ✓ | ✓ (heaviest) |
| Extension lifecycle | 14 | 6 | ✓ | ✓ |
| Channels (Slack/Telegram/webhook) | 5 | 3 | ✓ | ✓ |
| Triggers / automations / routines | 10 | 2 | ✓ | ✓ |
| Memory & workspace | 11 | 2 | — | ✓ |
| Skills | 1 | 1 | — | ✓ |
| Multi-user / scope isolation | 5 | 4 | 9 | ✓ |
| Tools & tool dispatch | — | 11 | ✓ | ✓ |
| Turn lifecycle (cancel/steer/retry/restart) | — | 8 | ✓ | ✓ |
| WebUI surfaces & APIs | 2 | 2 | — | ✓ (largest) |
| Durability & restart | 4 | 6 | ✓ | ✓ |
| Security & redaction | — | 3 | ✓ | ✓ |
| Providers (Google/Slack/GitHub contracts) | — | — | ✓ | ✓ |
| Coverage/meta gates | — | 2 | ✓ | ✓ |

Totals: **61** group scenarios · **62** flat integration bins (55 in
`tests/integration/`, 7 in `tests/integration/auth/`) · **39** top-level Rust bins ·
**103** Python scenario files (**889** test functions) registered in the active
Reborn coverage map below. Section 6 separately inventories retained and legacy
Python scenarios, so its exhaustive totals are intentionally broader.

---

## 3. Group scenarios — `tests/integration/group_*/` (61)

Multi-thread journeys over ONE shared runtime and ONE shared set of stores. These are
the canonical "a user does X in one conversation and sees the effect in another" tests.

### 3.1 Approvals — `group_approvals/` (10)

| The user can… | Evidence |
|---|---|
| Be asked to approve a risky file write, approve it, and watch the run finish | `scenario_gate_then_approve.rs` |
| Deny that same prompt and get a real answer back (not a hang) | `scenario_gate_then_deny.rs` |
| Say "always allow" once and never be asked again — including in a *different* conversation | `scenario_approve_always_persists_cross_thread.rs` |
| Have a tool marked "ask each time" prompt once per use and resume in one round trip (not re-prompt itself into a loop) | `scenario_ask_each_time_resumes_once.rs` |
| Approve/deny by replying in the channel ("approve"/"deny") rather than clicking a button | `scenario_submit_inbound_approval_resolution.rs` |
| Have two conversations waiting on approval at once, resolve them oppositely, and neither disturbs the other | `scenario_concurrent_dual_gate_resume.rs`, `scenario_concurrent_dual_gate_resume_parallel.rs` (also runs on libSQL) |
| Close the app with a prompt pending and still find it pending after reopen | `scenario_approval_request_persists_after_reopen.rs` |
| See the real reason a run failed instead of a generic "protocol violation" | `scenario_failure_category_demasked.rs` |
| Not resolve a gate with a stale/bogus reference, or resolve a gate that isn't there | `scenario_gate_ref_edge_cases.rs` |

### 3.2 Extensions — `group_extensions/` (14)

| The user can… | Evidence |
|---|---|
| Install an integration in one chat and see it active in another | `scenario_install_then_visible_cross_thread.rs`, `scenario_install_then_active_cross_thread.rs` |
| Remove an integration and have it gone everywhere | `scenario_remove_then_absent_cross_thread.rs` |
| Not call an integration's tools until it is actually installed | `scenario_uninstalled_tool_call_denied_until_active.rs` |
| Ask for an integration that doesn't exist and get a clear error rather than a crash | `scenario_install_unknown_extension_id_fails_safely.rs` |
| Get field-level repair guidance when the agent sends malformed install arguments | `scenario_malformed_lifecycle_arguments_are_structured.rs` |
| Be sent to a normal per-account sign-in when installing GitHub (no false "operator must configure this" wall) | `scenario_extension_install_github_normal_gate.rs` |
| Be told early and cleanly that Google isn't configured on this deployment — and sail through once an operator configures it | `scenario_extension_install_instance_not_configured.rs` |
| Be re-prompted to sign in when the provider revoked the grant, without a misleading tool error | `scenario_extension_install_reauth_gate.rs` |
| Install several Google apps and get an independent sign-in prompt per app, with the right scopes (a Gmail-only grant does not silently unlock Calendar) | `scenario_google_family_install_gate_and_shared_account.rs` |
| Finish personal setup and have their membership flip to active without a separate "activate" step | `scenario_existing_member_reinstall_reconciles_to_active.rs` |
| Connect Slack, use it, disconnect, reconnect, and use it again — with every surface (connection state, tools, page state) agreeing after each step | `scenario_slack_channel_lifecycle_state_machine.rs` |
| Restart the service and find Slack still connected | `scenario_slack_state_survives_reopen.rs` |
| Do the same install → use → remove → reconfigure → reinstall cycle for a credential-backed (GitHub) extension | `scenario_credential_extension_lifecycle_state_machine.rs` |

### 3.3 Multi-gate user journeys — `group_journeys/` (6)

| The user can… | Evidence |
|---|---|
| Approve a write, then deny a write, then keep chatting — with all three turns' history intact on one conversation | `scenario_interactive_approval_journey.rs` |
| Hit an approval prompt and a sign-in prompt in the same tool call, resolve both, then keep going across turns | `scenario_auth_then_approval_journey.rs` |
| Hit a sign-in prompt with no approval in the way, paste a token, and have the parked tool run for real | `scenario_auth_gate_grant_resume.rs` |
| Decline a sign-in, then sign in successfully later on the same conversation | `scenario_auth_deny_then_retry_journey.rs` |
| Have a stored-but-expired credential rejected, reconnect, and have the tool retry **with the new credential** | `scenario_expired_credential_resume.rs` |
| Not resolve another person's approval prompt — each user answers their own | `scenario_multi_actor_gate_isolation.rs` |

### 3.4 Memory — `group_memory/` (11)

| The user can… | Evidence |
|---|---|
| Have the assistant write a note in one chat and read it back in another | `scenario_write_then_read_cross_thread.rs` |
| Search memory from a different conversation and find what was written | `scenario_memory_search_finds_seeded.rs` |
| See the real folder structure of their memory | `scenario_memory_tree_reflects_structure.rs` |
| Run a build with memory disabled and have the assistant not even see memory tools | `scenario_disabled_binding_offers_no_memory_tools.rs` |
| Trust that only the memory hooks the provider declares actually fire | `scenario_lifecycle_gates_host_memory_calls.rs` |
| Ask a natural punctuated question in a new chat and receive explicitly saved memory — and only your own, never another user's, and never another conversation's raw transcript — framed as a recollection to verify (#7294), through the proactive prompt lane on the shipping libSQL backend | `scenario_proactive_prompt_recall_libsql.rs` |
| Have the assistant remember a preference you mentioned in passing and still know it in a later chat that opens on a completely unrelated subject — full-text retrieval cannot cover this, because it matches on the current message's words | `scenario_always_on_memory_recall_libsql.rs` |
| Ask about a stored fact in their OWN words rather than the words it was saved in, and still have it recalled — the paraphrase shares only some of the saved sentence's terms and adds one it never contained (#7185) | `scenario_paraphrased_prompt_recall_libsql.rs` |
| Tell from the run itself that memory retrieval BROKE rather than simply having nothing to say — the two used to be the same silent empty section (#7185/#7275) | `scenario_memory_retrieval_failure_is_visible.rs` |
| Have their standing memory document tidied for them every so often — redundant entries merged, superseded facts resolved — by a background pass that runs with nobody present, as them and only them (#7276) | `scenario_memory_curation_rewrites_standing_document.rs` |
| Not pay for that tidying on every single turn: below the configured interval no pass is submitted at all (#7276) | `scenario_memory_curation_below_threshold_never_fires.rs` |

### 3.5 Multi-user — `group_multiuser/` (5)

| The user can… | Evidence |
|---|---|
| Mention the bot in a shared channel and have the run act as the pinger — a second, distinct actor's run acts as itself, never the first binder (owner == actor) — while a Direct-route probe of the shared conversation is refused. Ephemeral-per-ping thread minting is pinned at the conversations tier and the channel e2e. | `scenario_shared_route_refuses_direct_reclassification.rs` |
| Not read another user's memories (and still read their own) | `scenario_memory_isolation_across_actors.rs` |
| Not have another user's "always allow" apply to them | `scenario_auto_approve_isolation_across_actors.rs` |
| Not see another user's run/turn state | `scenario_turn_state_isolation_across_actors.rs` |
| On a per-caller-scoped (served) deployment: read their brand-new workspace as empty instead of an error, approve a gated write that lands in their own `tenants/{tenant}/users/{user}` subtree (never the shared root), and read it back — while a missing sub-path stays a hard error | `scenario_scoped_workspace_isolation.rs` |

### 3.6 Skills — `group_skills/` (1)

| The user can… | Evidence |
|---|---|
| List, install, and remove a skill, with each step visible from a different conversation | `scenario_install_list_remove.rs` |

### 3.7 Linked accounts (device link) — `group_device_link/` (4)

Telegram's **linked-account** surfaces: the real bundled manifest (channel +
`method = "device_link"` auth + fifteen `standard_op` tools), its
`[admin_configuration]` satisfied through the production capability (including
the MTProto `telegram_api_id` / `telegram_api_hash`), and all three surfaces
bound through the same native-factory seam the binary uses. Only the vendor half
is scripted (a `DeviceLinkAdapter` and a linked-account `ToolAdapter`) — the real
ones speak MTProto over a raw socket with no injectable seam.

| The user can… | Evidence |
|---|---|
| Configure the deployment, install Telegram, link their own account, have the assistant read it through a real tool call — and lose that tool the moment the link is revoked | `scenario_link_call_unlink.rs` |
| Link their own Telegram account without inheriting (or leaking) someone else's — a second person's call acts as themselves | `scenario_actor_isolation.rs` |
| Have a revoked link park the run on a connect prompt instead of failing silently, then re-link and have the parked call run for real | `scenario_revoked_session_reauth.rs` |
| Link their account through the real multi-step handshake — scan, wait, type the account password — keep that personal credential separate from workspace-bot identity and pairing, use it immediately through the assistant, then remove the extension and have the provider device disappear | `scenario_handshake_mints_and_serves.rs` (drives the production `DeviceLinkFlowDriver`: start → poll → submit → completed, asserts the minted account's §4.5 ownership pin and durable custody, proves neither linking nor installing personal-account tools creates a bot-channel identity, proves a linked tool call resolves to that account, then removes the extension through the production lifecycle and observes the scripted provider revoke) |

### 3.8 Triggers & automations — `group_triggers/` (10)

| The user can… | Evidence |
|---|---|
| Create a scheduled automation, then pause/resume/remove it from another conversation | `scenario_verbs_lifecycle.rs` |
| Restart the service and still have their automation | `scenario_trigger_persists_after_reopen.rs` |
| Have a scheduled run ask for approval mid-fire and resume after they answer | `scenario_triggered_gate.rs` |
| Have a scheduled run chain through *two* approval prompts and still be recognised as a scheduled run | `scenario_triggered_chained_gate.rs` |
| See "waiting on you" on an automation whose run is parked, and see it clear when the run ends | `scenario_triggered_gate_hold_visible.rs` |
| Trust that a scheduled run can't create/remove/pause its own automations | `scenario_trigger_self_create_denied.rs` |
| Have a scheduled run that repeatedly asks the absent user a question fail truthfully after two bounded nudges while retaining every rejected reply | `scenario_scheduled_final_output.rs` |
| Create an automation whose create input carries no delivery-routing field at all | `scenario_trigger_create_has_no_delivery_target_field.rs` |
| See and rename their automations in the WebUI, backed by the real trigger store | `scenario_webui_automations_list.rs`, `scenario_webui_automations_rename.rs` |

---

## 4. Flat integration bins — `tests/integration/*.rs` and `tests/integration/auth/*.rs` (62)

One thread, whole real turn. Grouped by what the user experiences.

**Chat & turn lifecycle**
| Behavior | Evidence |
|---|---|
| A plain message gets a persisted reply through the whole real stack | `greeting.rs` |
| Stopping a running turn actually stops it (Cancelled, not Completed) | `cancel.rs` |
| Typing again while the assistant is working queues the message and it gets picked up mid-run | `steering.rs` |
| A flaky model provider is retried and recovered from, with typed errors | `model_recovery.rs` |
| Incremental compaction summaries preserve every disjoint compacted range while raw covered history remains stored | `model_recovery.rs::compaction_summary_chain_preserves_earlier_compacted_history` |
| A turn receives the expected tool results after each model iteration | `golden_payload.rs` |
| A turn that reads two file ranges in parallel receives both results in the requested order | `golden_payload.rs` |
| Approaching the run limit surfaces a recoverable warning, while repeated capability calls receive one advisory warning and may continue | `terminal_warning.rs` |
| Repeating the same inbound message does not start a second run | `idempotent_replay.rs` |
| Spend accounting fires on a real turn | `budget.rs` |
| Sub-agents spawn and awaiting them behaves at the edges | `subagent_await_edge.rs` |
| Two background children settle independently while the parent keeps running: each gets its own framed transcript row and its own queued `SubagentSettled` input, in settle order (D6) | `subagent_await_edge.rs::background_child_result_is_delivered_per_child_while_parent_runs` |
| A background result's live-run enqueue racing `RunClosed` (the parent terminalized mid-delivery) is healed by a System-provenance `activate`, not lost | `subagent_await_edge.rs::run_closed_race_is_healed_by_activation` |
| A background result settling against a parked/completed parent wakes it via `activate` with `ActivationProvenance::System`, pinned on the submitted run's journaled `subagent_activation_provenance` | `subagent_await_edge.rs::parked_parent_is_activated_with_system_provenance` |
| Re-driving background delivery after a scripted crash mid-append replays idempotently — exactly one transcript row and one queued attention outcome, never two | `subagent_await_edge.rs::background_delivery_replay_is_idempotent` |
| A background result parked by the autonomous-wake streak cap (`AttentionDeferredStreakCap`) stays unclosed and immune to autonomous re-drive until a human-provenance run start sweeps, drains, and closes it | `subagent_await_edge.rs::streak_capped_result_waits_for_human` |
| A caller hands the engine a prepared prompt and gets its outcome back: a schema-validated JSON result (invalid attempts are retried and the corrected payload is durably recorded) or a plain answer; seeded tool history is honored by the run; resubmitting the same request is replay-safe; the private work thread belongs to the calling user (stored under their owner scope, foreign-owner run-state reads rejected) yet never appears in conversation listings | `unbound_turns.rs` |

**Suggestions**
| Behavior | Evidence |
|---|---|
| Generate suggestion cards and replay the same client action without starting another generation | `suggestions.rs::generate_suggestions_returns_cards_and_cached_replay` |
| Replace the visible suggestion cards with a new generation while preserving existing start reservations | `suggestions.rs::replacement_generation_preserves_reservations_and_replaces_cards` |
| Start a suggestion card and create one canonical thread containing its suggested prompt | `suggestions.rs::starting_a_replacement_suggestion_creates_one_thread` |
| Keep suggestion cards and their start/dismiss actions isolated to the authenticated tenant and user scope | `suggestions.rs::suggestions_are_isolated_by_authenticated_scope` |
| Dismiss a started suggestion across restart without deleting its thread or timeline | `suggestions.rs::dismissing_a_started_suggestion_persists_across_restart` |
| Settle failed or contract-invalid suggestion generation as failed with no visible cards | `suggestions.rs::failed_suggestion_run_settles_failed_and_retryable_via_list_view`, `suggestions.rs::semantically_invalid_completed_suggestion_output_settles_failed`, `suggestions.rs::unknown_field_in_completed_suggestion_output_settles_failed` |
| Reach the user's connected extension tools in an autonomous run under the system default, and lose exactly one of them to a per-tool override — the only row of these six needing disclosure forced off, since two connected extensions' full authorized set would otherwise defer behind `tool_search` | `suggestions.rs::connected_extensions_are_reachable_under_the_system_default` |
| Shrink the autonomous surface when the user turns the global auto-approve toggle off, leaving only the synthetic capabilities that bypass surface policy — a distinct settings-store write from the per-tool override rows below, and the baseline the excluded-call refusal row reuses | `suggestions.rs::disabling_global_auto_approve_shrinks_the_autonomous_surface` |
| Remove exactly one capability from an autonomous run when the user sets it to ask-each-time — a per-tool override rather than the global toggle above, and a different override value than the disabled row below | `suggestions.rs::per_tool_ask_each_time_override_removes_only_that_tool` |
| Keep a disabled capability out of an autonomous run even while global auto-approve is on — `Disabled` outranks the default-on toggle, a precedence ask-each-time doesn't claim | `suggestions.rs::per_tool_disabled_override_removes_the_tool_even_with_auto_approve_on` |
| Reach a connected extension's capability in an autonomous run only through the user's own always-allow grant when auto-approve is off — combines a connected extension, the global toggle, and an explicit persistent grant, a fixture no single row above exercises | `suggestions.rs::connected_gmail_tool_is_reachable_only_via_the_users_own_grant` |
| Refuse rather than dispatch a capability the autonomous surface excluded, when the model calls it anyway — proves the model-gateway dispatch seam, not just the advertised tool list the rows above check | `suggestions.rs::excluded_capability_call_is_refused_not_dispatched` |

**Tools**
| Behavior | Evidence |
|---|---|
| An HTTP tool call reaches the real egress boundary and the result reaches the model | `tool_call.rs`, `http_matcher.rs` |
| Saved, transcript-shaped JSON can be queried through scoped storage with plain or `$`-rooted paths; bounded collection operations can select the last item and aggregate numeric rows; invalid JSON produces model-visible correction guidance | `tool_call.rs` |
| Inspect a >100 KiB nested JSON capability result through bounded first-look, node/scalar, collection, credential-redaction, invalid-selection, and exact legacy-byte views | `tool_call.rs::{result_read_large_nested_result_first_look_is_bounded_and_parseable,result_read_selects_nested_json_node_and_scalar,result_read_pages_nested_json_collection,result_read_redacts_credential_json_within_requested_budget,invalid_json_result_selections_remain_model_correctable,result_read_preserves_exact_legacy_byte_reads}` |
| Shell commands dispatch through the real path without spawning an OS process | `process_port.rs` |
| A sandbox-profile shell turn executes as an unprivileged user in one reusable per-user Docker container, preserving workspace and container-local state across shell calls and sharing that container across the user's threads | `reborn_sandbox_shell_turn.rs` |
| MCP tools work over a real loopback HTTP MCP server | `mcp.rs` |
| User-registered and bundled hosted MCP servers register, authenticate, project active, restore, and invoke | `hosted_mcp_registration.rs` |
| Web search/fetch runs the real Exa MCP handshake | `web_access.rs` |
| Outbound HTTP crosses the real security pipeline (network policy + leak scan) | `real_egress_pipeline.rs` |
| Tools marked host-internal are never advertised to the model, and calls to them are rejected | `extension_visibility.rs`, `surface_disclosure.rs` |
| Ordinary authentication vocabulary in verified and locally imported tool descriptions survives prompt construction without denying the turn | `extension_visibility.rs::prompt_description_auth_vocabulary_survives_at_the_real_turn_seam` |
| With a large tool catalog, progressive-disclosure modes and the `namespaces` production default expose `tool_search`, `tool_describe`, and `tool_call` instead of flat tools; a complete search signature invokes directly, while incomplete or explicitly inspected results fall back through `tool_describe` | `tool_disclosure.rs` |
| Deferred tools can be found from argument-only vocabulary without adding that schema vocabulary to the model prompt | `tool_disclosure.rs::tool_search_discovers_authorized_tools_by_parameter_only_vocabulary` |
| Bridged disclosure never reintroduces host-runtime capability metadata excluded by any resolved host-API surface-policy dimension (ID, runtime, effect, approval, or maximum count) | `tool_disclosure.rs` |
| A capability whose lease expires mid-dispatch does not wedge the run | `lease_wedge.rs` |
| A run whose lease expires while it is waiting on the model finishes normally instead of dying — it is resumed from its before-model checkpoint after a grace window, and the user never sees a failure | `lease_wedge.rs::run_parked_before_a_model_call_is_resumed_after_lease_expiry_not_failed` |
| Attachments the user uploads are read back byte-for-byte by the model | `attach.rs` |
| Uploaded DOCX files cannot be corrupted by raw text writes; structured DOCX/XLSX/PPTX edits produce new downloadable files without changing the originals; and HTML renders to a persisted PDF | `document_edit.rs` |
| Skill activation injects skill context into a real turn | `skill_activate.rs` |
| Creating a project through chat persists it | `project_create.rs` |
| Profile writes reach the real profile source | `profile.rs` |
| Delivery-target tools resolve through the real outbound service | `outbound_target.rs` |

**Auth** (`tests/integration/auth/`)
| Behavior | Evidence |
|---|---|
| A full OAuth connect → callback → stored account round trip | `auth/oauth_connect.rs` |
| Abandoning the OAuth popup, late callbacks, and retrying cleanly | `auth/oauth_popup_journeys.rs` |
| Idle credentials get refreshed by the background sweep | `auth/oauth_refresh.rs` |
| A missing credential parks a sign-in gate; denying it ends the run cleanly | `auth/auth_gate.rs` |
| A revoked/`invalid_grant` credential is marked revoked and read back as such | `auth/auth_failure.rs` |
| The service restarts while a gate is pending, and approving afterwards still resumes the run | `auth/reopen_resume_through_gate.rs` |
| Credentials are injected into the real outbound request (proof they reach the wire) | `secret_injection.rs` |

**Extensions & channels**
| Behavior | Evidence |
|---|---|
| An extension installs and activates through the real generic runtime | `extension_runtime.rs` |
| An inbound channel message is verified and routed by the real generic ingress mount | `extension_ingress.rs` |
| An outbound reply is delivered through the real inbound→outbound pipeline | `extension_delivery.rs` |
| Pair a Telegram bot actor, run as that verified user, deliver anchored replies and busy notices, disconnect to revoke admission, then pair again to restore delivery (#6643/#6644) | `extension_delivery.rs::paired_telegram_bot_actor_turns_attribute_to_the_user_and_disconnect_revokes_admission` (production generated-code pairing, disconnect/repair, and anchored delivery evidence) |
| Tenant-admin configuration and per-user install/remove stay separate state machines | `extension_user_lifecycle_isolation.rs` |
| A notification inbox belongs to one recipient: knowing another user's notification id grants no read and no mutation | `notification_inbox_user_isolation.rs` |
| Connect Telegram through a generated workspace-bot code while personal linked-account tools remain separately protected | `channel_connection_projection.rs` |
| An ordinary Telegram user sees personal device-link setup without deployment secrets and can independently mint a workspace-bot pairing code | `webui_v2_product_api.rs::telegram_setup_separates_bot_pairing_from_personal_device_link` |
| Delivery preferences / connected channels render into the model prompt | `comm_context.rs` |

**Durability, storage & restart**
| Behavior | Evidence |
|---|---|
| Behavior is identical on in-memory, libSQL, and PostgreSQL storage; message exact lookups avoid sibling entry rows on both durable databases | `backend_matrix.rs` |
| Installed extensions survive a fresh store reopen | `durable.rs` |
| Secrets survive a genuine on-disk reopen | `secrets.rs` |
| Outbound preferences survive a process-level reopen | `outbound_store_durability.rs` |
| Restart sequences over a gated run recover correctly | `generated_restart_sequences.rs` |
| A suggestions generation survives a backend restart: GET/list alone shows it generating and then ready after durable recovery | `suggestions.rs::generation_in_progress_survives_runtime_restart_and_recovers_via_list_view` |
| Odd gate sequences (double-resolve, cancel-after-finish, approve-a-done-run) behave | `generated_gate_sequences.rs` |

**Platform / wiring**
| Behavior | Evidence |
|---|---|
| Lifecycle hooks fire at the expected points and a hook deny blocks the tool without wedging | `hooks.rs` |
| Turn lifecycle events publish to subscribers; trace capture records them | `tracecap.rs`, `trace_capture.rs` |
| Instruction-safety context renders into the prompt | `safety.rs` |
| Scheduled-origin runs carry their origin into persisted state | `triggered_submit.rs` |
| The test harness's runtime wiring stays field-identical to production's | `wiring_parity.rs` |
| WebUI v2 routes work over the real services facade | `webui_v2_product_api.rs`, `webui_v2_router_smoke.rs` |
| Enroll/refresh/remove a browser for web push over the real routes — advertised VAPID key, endpoint redacted to its push-service host, undeclared push hosts rejected, and the `web-app` catalog row selectable through the same notification-channels wire as every vendor channel | `webui_v2_product_api.rs::browser_channel_notification_setup_round_trip_through_production_facade` |
| Identity resolution runs on the coverage lane | `identity_resolution_smoke.rs` |
| A canonical 10-tool-call agent turn's database write volume is measured and reported (for tracking, not gated) on both libSQL and Postgres, and custom-actor group threads are rejected from canonical durable milestones | `db_write_canonical.rs` |
| A downloaded run artifact carries per-iteration model-call timing evidence for a completed run, and still carries durable per-message timestamps (with an explicit `run_not_resident` reason) when the process-local timing buffer was evicted or the process restarted | `run_artifact_timings.rs` |

One of the 62 registered bins, `delivery_user_journeys.rs`, holds the explicit
channel-delivery journeys (two-lane model):

| A user can… | Scenario |
|---|---|
| Ask in WebUI to be pinged on Slack and get a bot delivery with provider evidence | `webui_send_me_on_slack_delivers_via_bot_with_evidence` |
| Ask from Slack for a Telegram delivery; ack lands in Slack, payload in Telegram | `slack_origin_delivers_to_telegram_and_acks_in_slack` |
| Never have the model deliver into the run's own conversation (lane 1 is automatic) | `deliver_to_origin_conversation_is_denied_and_model_replies` |
| See per-call honest results when one of several deliveries fails | `partial_failure_reports_per_call_honestly` |
| Get a refusal (no tool calls) for an undeliverable destination | `undeliverable_destination_is_refused_without_tool_calls` |
| Have a routine fire deliver via the tool with no stored target anywhere | `routine_fire_delivers_via_tool_without_stored_target` |
| Have a conditional fire that calls no delivery tool produce zero outbound attempts | `conditional_fire_with_no_delivery_call_produces_zero_attempts` |
| Get blocked-fire notices fanned out to every notification channel, first approve wins | `blocked_fire_fans_out_and_first_approve_wins` |
| Keep a blocked fire app-only when the notification-channel set is empty | `empty_notification_set_keeps_blocked_fire_in_app_only` |
| Enroll a browser and get a blocked fire's gate notice as a real Web Push (encrypted `aes128gcm` body, host-injected `Authorization: vapid`, one POST to the enrolled endpoint) while the run stays parked | `blocked_fire_pushes_web_app_notice_to_enrolled_browser` |
| Have a dead browser subscription (push service answers `410 Gone`) pruned after one notice attempt | `gone_push_subscription_is_pruned_after_notice_attempt` |

---

## 5. Binary, parity & QA-trace bins — `tests/*.rs` (39)

**QA workflow phrases** — real manual-QA sentences, replayed against the Reborn binary.
| The user asks… | Evidence |
|---|---|
| "connect to <service>" and completes the auth flow | `reborn_qa_connect_flows.rs` (8) |
| "every 30 minutes email me a summary…" and gets a routine | `reborn_qa_routines.rs` (7) |
| a question in a Slack DM / a keyword-prefixed message, and gets a Slack reply | `reborn_qa_channel_delivery.rs` (2) |
| "use this Google Drive doc as your knowledge base" | `reborn_qa_doc_grounding.rs` (2) |
| "does api.github.com return 200 / summarize this release page" | `reborn_qa_web_fetch.rs` (3) |
| any of the above, replayed from committed real-LLM traces | `reborn_qa_recorded_behavior.rs` (29) |
| "send me XYZ every morning" from the web app and gets a routine whose fires stay in the run thread (no delivery step — the source-surface default, live-recorded) | `reborn_qa_recorded_behavior.rs::contract_routine_bare_send_me_from_web_app_pins_no_delivery_step` (+ replay) |
| "send me XYZ to Slack and Telegram" and gets a routine whose prompt pins one delivery step per channel (live-recorded) | `reborn_qa_recorded_behavior.rs::contract_routine_multi_channel_delivery_pins_both_targets_in_prompt` (+ replay) |
| a broad smoke set of whole turns on a full runtime | `reborn_qa_smoke_scenarios_e2e.rs` (10) |

**Binary-level behavior**
| Behavior | Evidence |
|---|---|
| A provider failure yields a sanitized, *retryable* failure and the user can retry and resume | `reborn_failure_retry_resume_e2e.rs` (6) |
| Sub-agents spawn end-to-end | `reborn_subagent_spawn_e2e.rs` (5) |
| The shipped Docker image has a usable runtime home and an in-worker public-key SSH shell | `dockerfile_runtime_home.rs` (21) |
| Live GitHub API contracts still hold (ignored canary, needs a real PAT) | `reborn_live_github_pat_contract.rs` |

**Scope isolation parity** — one bin per boundary; each proves data from one scope is
unreachable from another: `reborn_agent_scope_isolation_parity.rs`,
`reborn_project_scope_isolation_parity.rs`,
`reborn_identity_{tenant,project,prompt}_scope_isolation_parity.rs`,
`reborn_tenant_binding_scope_isolation_parity.rs`,
`reborn_thread_binding_isolation_parity.rs`,
`reborn_direct_chat_user_scope_isolation_parity.rs`,
`reborn_http_network_scope_isolation_parity.rs`,
`reborn_adapter_installation_scope_isolation_parity.rs`,
`reborn_wrong_scope_access_isolation_parity.rs`.

**Trace parity** — recorded-trace equivalence for tool families and error paths:
`reborn_trace_core_builtin_tools_parity.rs`, `reborn_trace_file_tools_parity.rs`,
`reborn_trace_coding_read_tools_parity.rs`, `reborn_trace_error_path_parity.rs`,
`reborn_trace_wasm_github_fixture_parity.rs`,
`reborn_trace_first_party_tool_coverage.rs` (10; including model-visible trigger
status reads, product-triggered manual runs, and scheduled-run denial evidence),
`reborn_recorded_trace_parity.rs`, `reborn_minimal_dispatch_parity.rs`,
`reborn_response_order_parity.rs`, `reborn_tool_param_coercion_parity.rs`,
`reborn_approval_traces_parity.rs`, `reborn_turn_state_lock_free_submit_parity.rs`.

**Policy & format**: `e2e_trace_runtime_policy_org_ceiling_yolo.rs` (org policy
ceiling × yolo narrowing), `e2e_trace_runtime_policy_serde.rs` (wire-stable policy
enums), `trace_format.rs`, `trace_llm_tests.rs`,
`reborn_coverage_lane_stack_headroom.rs` (CI job must declare stack headroom).

---

## 6. Python E2E scenarios — `tests/e2e/scenarios/` (103 files, 1180 top-level tests)

This is an exhaustive inventory, not a claim that every retained scenario is
currently executable. Current Reborn coverage starts `ironclaw serve` through the
`reborn_v2_*` fixtures or exercises a current provider/API harness. Any cited test
that still uses `ironclaw_binary`, `ironclaw_server`, or the legacy `page`/`SEL`
fixtures is inventory-only, non-functional, and pending migration under #6369 —
even when it appears beside current Reborn evidence in the same row. See
`tests/e2e/AGENTS.md` §"Reborn E2E coverage gate" before adding coverage-manifest
entries.

### 6.1 Chat, shell & session
| The user can… | Evidence |
|---|---|
| Load the app, sign in with a token, navigate, and be rejected without one | `test_reborn_webui_v2_legacy_core.py`, `test_reborn_webui_v2_smoke.py::test_reborn_v2_serves_shell_and_gates_auth` |
| Send a message and see a streamed reply; empty messages don't send | `test_reborn_webui_v2_legacy_core.py::test_reborn_legacy_core_send_message_and_receive_response`, `…::test_reborn_legacy_core_empty_message_not_sent` |
| See their message immediately, keep it through a reconnect/reload, and not see it duplicated once confirmed | `test_reborn_webui_v2_legacy_pending_messages.py` (12), `test_pending_user_messages.py` (8) |
| Reload the page and still see history, tool cards, and in-progress turns | `test_reborn_webui_v2_legacy_sse_history.py` (10), `test_reborn_webui_v2_legacy_message_persistence.py`, `test_message_persistence.py` (10) |
| Type a draft while a run is processing | `test_reborn_webui_v2_smoke.py::test_reborn_v2_composer_accepts_draft_while_run_is_processing` |
| Click "+ New" or open a thread and start typing immediately — focus lands in the composer without a second click | `test_reborn_webui_v2_smoke.py::test_reborn_v2_composer_takes_focus_from_sidebar_navigation` |
| Start a new chat while a run is active (the #5256 deadlock regression) | `test_reborn_webui_v2_smoke.py` |
| Page through older messages/threads without losing scroll position | `test_reborn_webui_v2_smoke.py::test_reborn_v2_timeline_pagination`, `…::test_reborn_v2_loading_older_messages_preserves_viewport` |
| Keep DOM bounded on huge histories, without SSE timer leaks | `test_reborn_webui_v2_legacy_dom_resource_limits.py` (4) |
| Copy a message, use the command palette | `test_reborn_webui_v2_legacy_chat_actions.py` (3) |
| Delete a thread behind a shared confirmation dialog | `test_reborn_webui_v2_smoke.py::test_reborn_v2_thread_delete_uses_shared_confirmation_dialog` |
| Collapse the sidebar, collapse and reopen each expandable navigation section, and pick a theme that persists | `test_reborn_webui_v2_smoke.py::test_reborn_v2_desktop_sidebar_can_collapse_and_persist`, `test_reborn_webui_v2_smoke.py::test_reborn_v2_expandable_sidebar_sections_can_collapse`, `test_reborn_webui_v2_smoke.py::test_reborn_v2_appearance_theme_selection_persists` |
| Opt into the inspector for the browser session, toggle it from the header icon, preserve its selected tab while closing, resizing, reloading, and reconnecting after visibility changes, explicitly disable it, and leave the ordinary chat shell unchanged when debug mode is off | `test_reborn_webui_v2_smoke.py::test_inspector_debug_activation_and_responsive_shell` |
| Inspect the bounded host-resolved prompt, ordered activity timeline, turn navigation, and model-call statistics for completed runs, including continued diagnostic observation while the panel is closed | `test_reborn_webui_v2_smoke.py::test_inspector_prompt_and_stats_render_host_diagnostics` |
| Reconnect SSE without gaps or duplicates; multiple tabs both get the reply; excess connections are rate-limited | `test_reborn_webui_v2_legacy_sse_history.py`, `test_reborn_webui_v2_streaming_run_control_api.py` (10) |
| Keep execution-only engine threads out of the chat sidebar while preserving deep-linked history | `test_v2_thread_visibility.py` (2; pending legacy migration #6369) |

### 6.2 Tools, approvals & auth prompts (UI)
| The user can… | Evidence |
|---|---|
| Run built-in tools, parallel calls, and multi-step chains and see the result | `test_reborn_webui_v2_legacy_tool_execution.py` (9), `test_v2_engine_tool_lifecycle.py` (6), `test_tool_execution.py` |
| Inspect safely bounded arguments and a visibly truncated 50 KiB output for a completed tool call | `test_reborn_webui_v2_tool_gates.py::test_reborn_v2_tool_turn_records_result_and_final_reply` |
| Recover when a tool fails, a tool call is truncated, or the model loops | `test_reborn_webui_v2_legacy_tool_execution.py`, `test_agent_loop_recovery.py` (4), `test_v2_engine_error_handling.py` |
| See the approval card with payload details, approve/deny, and see it disable while resolving | `test_reborn_webui_v2_legacy_approval.py` (8) |
| Not send messages while a gate is pending — but keep using other threads | `test_reborn_webui_v2_legacy_approval.py::test_reborn_legacy_pending_approval_does_not_block_other_thread` |
| Approve/deny/always from Slack or Telegram DMs, and resolve a channel gate from the web | `test_channel_approval_gates.py` (9) |
| Cancel an in-flight turn from the UI | `test_reborn_webui_v2_tool_gates.py::test_reborn_v2_cancel_in_flight_turn_ends_cancelled` |
| Paste a manual token in the auth card, with the token never landing in the DOM or history | `test_reborn_webui_v2_legacy_auth_flows.py` (13), `test_v2_github_pat_flow.py` (7) |
| Set per-tool permissions and "always approve", surviving reload and restart | `test_reborn_webui_v2_legacy_tool_permissions.py` (9), `test_tool_permissions.py` (6), `test_v2_engine_approval_flow.py` (7) |
| Receive one authentication gate without a duplicate instruction response | `test_auth_no_duplicate_response.py` (pending legacy migration #6369) |

### 6.3 OAuth & credentials
| The user can… | Evidence |
|---|---|
| Complete a Google/GSuite OAuth round trip with PKCE, redirect-URI and client-binding checks | `test_v2_gsuite_oauth_flow.py` (11), `test_v2_engine_oauth_google.py` (5) |
| Complete a Notion **MCP** OAuth round trip and have the bearer injected into `tools/call` | `test_v2_notion_mcp_oauth_flow.py` (13) |
| Complete OAuth for WASM tools, WASM channels and MCP servers, including provider-error, exchange-failure and replayed-callback paths | `test_v2_auth_oauth_matrix.py` (19), `test_extension_oauth.py` (10), `test_mcp_auth_flow.py` (11) |
| Have an expired token refreshed automatically via the hosted proxy, without leaking `client_secret` | `test_oauth_refresh.py` (4) |
| Cancel mid-auth, submit an empty/invalid token, and recover | `test_v2_engine_auth_cancel.py`, `test_v2_engine_auth_flow.py` (19) |
| Trust OAuth URLs are well-formed and identical across channels (bug #992) | `test_oauth_url_parameters.py` (8) |
| Have credentials stay per-user and per-thread scoped | `test_v2_kernel_auth_gateway_flow.py` (3), `test_skill_oauth_flow.py` (6), `test_oauth_credential_fallback.py` |
| Sign in with Google-shaped SSO and have two users keep separate threads and notification inboxes | `test_reborn_webui_v2_sso.py` |

### 6.4 Extensions & MCP (UI + API)
| The user can… | Evidence |
|---|---|
| Browse the registry, search, install, configure, remove and reinstall an extension | `test_reborn_webui_v2_legacy_extensions.py` (36), `test_extensions.py` (59), `test_wasm_lifecycle.py` (35) |
| Recover when the catalog fails, enrichment fails, install fails, or they're offline | `test_reborn_webui_v2_legacy_extensions.py` |
| Fill in a configure modal (all field variants, https-only setup URLs, focus trapping, enter-to-submit) | `test_reborn_webui_v2_legacy_extensions.py`, `test_extensions.py` |
| See the right button label for authed vs unauthed extensions (#2235) | `test_settings_extensions_labels.py` (5) |
| Register a custom hosted MCP server and install it with bearer or OAuth | `test_reborn_webui_v2_custom_mcp.py` (3) |
| Have uninstall delete that extension's secrets while preserving shared credentials | `test_extension_uninstall_cleanup.py` (4) |
| Install a private/imported tool and have per-user visibility respected | `test_reborn_private_tool_installs.py` |
| Drive the whole lifecycle over the API without a browser | `test_reborn_webui_v2_extensions_api.py` (4) |

### 6.5 Channels
| The user can… | Evidence |
|---|---|
| Set up Slack, DM the bot / @-mention it, get a threaded reply, and have bot/subtype messages ignored | `test_slack_e2e.py` (13) |
| Have Slack reject bad HMAC, missing headers, unauthorized users, malformed payloads | `test_slack_e2e.py` |
| Configure → connect → use → remove Slack through the *generic* channel routes | `test_reborn_slack_channel_e2e.py` |
| Set up Telegram (webhook or polling), DM it, edit a message, get long messages chunked, survive rate limits and bad payloads | `test_telegram_e2e.py` (13) |
| Activate Telegram from the UI and move through pairing states | `test_telegram_hot_activation.py` (6), `test_telegram_token_validation.py` |
| Pair a Telegram account by pasting the code in the right place (#3317) | `test_telegram_pairing_chat_claim.py` (4), `test_channel_pairing_flow.py` (6), `test_pairing.py` (7) |
| Send events over the HTTP webhook channel with HMAC-SHA256 auth | `test_webhook.py` (11) |

### 6.6 Automations, routines & projects
| The user can… | Evidence |
|---|---|
| Create an automation through chat; run it now, rename, pause, resume, reload, and delete it through the UI with persisted API state; disable run-now while a request is pending, a fire or run is active, a duplicate click is in flight, or the scheduler is off; keep the list visible while filtering; dismiss and clear safe mutation-error toasts; and open a failed run or its scoped logs | `test_reborn_webui_v2_smoke.py::test_reborn_v2_automation_lifecycle_persists_from_ui`, `…::test_reborn_v2_automation_run_now_respects_active_fire_and_scheduler`, `…::test_reborn_v2_automation_filter_keeps_list_visible_while_loading`, `…::test_reborn_v2_automation_action_error_toast_is_safe_dismissible_and_cleared_on_retry`, `…::test_reborn_v2_automation_failed_run_actions_are_clickable`, `test_reborn_webui_v2_automation_trace_outbound_api.py` (4) |
| Create event-triggered routines, have them fire on match, respect cooldown, pause/resume | `test_routine_event_batch.py` (8) |
| Run a full-job routine end-to-end with tools, trigger it manually, see failures in the UI | `test_routine_full_job.py` (3) |
| Have routines run with injected OAuth credentials | `test_routine_oauth_credential_injection.py` (3) |
| Create a Gmail-draft mission, pause it for authentication, and resume it after OAuth | `test_mission_gmail_3133.py` (pending legacy migration #6369) |
| Still reach their v1 routines after upgrading to v2 (#2982) | `test_routines_tab_after_v2_upgrade.py` (11) |
| Use the Missions tab instead of the removed Routines tab and activity strip | `test_v2_activity_shell.py` (2; pending legacy migration #6369) |
| See routines created in one surface from another (owner scope) | `test_owner_scope.py` (3) |
| Browse projects, create one, open a scoped chat, list and download workspace files | `test_reborn_webui_v2_legacy_projects.py` (6), `test_project_detail.py` (3), `test_reborn_v2_file_download.py` (4) |
| Open notifications with immediate feedback while its lazy chunk loads; read approval, authentication, completed, and failed notifications from the generic server-backed Inbox; drive real scheduled approval/auth gates through resolution; persist read state across a fresh client; mark one or all as read; wait for a matching final reply before acknowledging completion; and open the source thread | `test_reborn_webui_v2_notifications.py` (12) |

### 6.7 Settings, skills & admin
| The user can… | Evidence |
|---|---|
| Search across settings sections and clear the search | `test_reborn_webui_v2_legacy_settings_search.py` (6), `test_settings_search.py` (5) |
| As an admin, publish the active provider's allowlist/default from Settings; then, as a non-admin member, choose a long-name allowed model, verify the selector stacks at narrow width and right-aligns at wide width without overflow, and have that preference reach future provider requests across chats without changing another member's workspace-default routing | `test_reborn_webui_v2_smoke.py::test_reborn_v2_settings_model_preference_reaches_provider` |
| Add, test, activate, edit and delete a custom inference provider | `test_reborn_webui_v2_legacy_settings_search.py` |
| Add/edit/delete skills, with read-only sources locked | `test_reborn_webui_v2_legacy_skills.py` (3), `test_reborn_webui_v2_skills_api.py` (3) |
| Filter scoped logs by target and level with the shared SelectMenu while polling and pagination continue | `test_reborn_webui_v2_smoke.py::test_reborn_v2_logs_page_passes_scope_to_api_and_renders_context` |
| Use plan mode (`/plan`, checklist, approve, status) | `test_plan_mode.py` (5) |
| As an admin: create users, hand out one-time tokens, page the user list, set roles, suspend/activate, manage write-only secrets, delete users | `test_admin_api.py` (18) |
| Bootstrap as the single-tenant owner with stable identity | `test_ownership_model.py` (8), `test_multi_tenant_greeting.py` |
| Customize their gateway layout/widgets by chatting | `test_widget_customization.py` (4) |

### 6.8 APIs (OpenAI-compatible, filesystem, operator)
| The user can… | Evidence |
|---|---|
| Use the Responses API (non-streaming, streaming SSE, continue, retrieve, context injection, auth/validation errors, and successful external output round-tripping through `result_read`) | `test_reborn_responses_api.py` (15), `test_reborn_webui_v2_legacy_responses_api.py` (14), `test_responses_api.py` (9) |
| Use chat-completions with idempotency replay and streaming | `test_reborn_responses_api.py` |
| Mix internal and external tools in one response and get failures back to the LLM | `test_reborn_responses_api.py` |
| List thread files and browse filesystem mounts, with path traversal rejected | `test_reborn_webui_v2_filesystem_api.py` (3) |
| Drive sessions/threads/messages over the API | `test_reborn_webui_v2_session_api.py` (3) |
| Configure LLM providers and read operator status/logs | `test_reborn_webui_v2_operator_api.py` (3) |
| Manage product-auth accounts with redacted errors | `test_reborn_webui_v2_product_auth_api.py` (4) |

### 6.9 Robustness, security & durability
| The user can… | Evidence |
|---|---|
| Kill the process (`kill -9`) and still find their history | `test_reborn_blackbox_smoke.py` (5) |
| Restart gracefully and keep thread history and "always approve" | `test_reborn_blackbox_smoke.py`, `test_reborn_webui_v2_legacy_tool_permissions.py::test_reborn_legacy_always_approve_survives_reborn_restart` |
| Not be XSS'd by assistant or user content | `test_reborn_webui_v2_legacy_rendering.py` (3), `test_csp.py` (4) |
| Upload and reselect attachments within count/size/type limits, with extraction and placeholders | `test_reborn_webui_v2_legacy_attachments.py` (8) |
| Have the model retried on HTTP errors and broken SSE streams, and cancel a slow inference | `test_reborn_webui_v2_streaming_run_control_api.py` |

### 6.10 Provider contracts & fixtures (Emulate-backed)
| What is proven | Evidence |
|---|---|
| Gmail/Calendar/Drive/Docs/Sheets/Slides, Slack and GitHub read+write contracts against a real provider emulator | `test_emulate_reborn_provider_contracts.py` (14), `test_reborn_emulate_full_path.py` (5) |
| Harvested live-QA traces replay through the served binary and mutate real provider state | `test_reborn_qa_trace_full_path.py` (6), `test_reborn_qa_trace_replay.py` (19) |
| Provider faults (status, timeout, connection reset, truncated body, lost ack, credential lifecycle) behave safely and never leak credentials | `test_provider_fault_proxy.py` (18), `test_provider_operation_types.py` (4) |
| Each journey gets a clean provider world (reset really resets) | `test_provider_world_isolation.py` (9) |
| Local runs use the same Emulate build CI does | `test_emulate_build_parity.py` (5) |

### 6.11 Coverage gates (meta-tests — these are how §7 stays honest)
| What is proven | Evidence |
|---|---|
| Every shipped first-party provider capability is tested, live-only, unsupported, or has an owned waiver | `test_provider_capability_inventory.py` (18) + `fixtures/provider_capability_coverage.toml` |
| Every supported ingress/delivery surface names exact executable evidence; manifests can't add a surface without it | `test_journey_coverage.py` (36) |
| Every lifecycle state-machine claim names executable evidence, with zero gaps | `test_state_machine_coverage.py` (12) |
| The product-surface coverage report can't pass vacuously or hide lost evidence | `test_product_surface_coverage.py` (7) |
| Playwright diagnostic artifacts stay bounded | `test_reborn_webui_harness_artifacts.py` (10) |

> These gates fail loudly when coverage regresses. Prefer adding a row to one of their
> registries over adding a line to §7.

---

## 7. Known gaps

Derived from the inventory above. "Gap" here means *no scenario-tier test exists*; some
of these are covered at crate tier, which is a weaker but real signal — say so if you
verify it.

| Gap | Notes |
|---|---|
| **Proactive / background execution** has no Reborn scenario at any tier | The v1 heartbeat loop has no Reborn equivalent yet — issue #6369. Nothing in §3–§6 drives it. |
| **Skills** have only one group scenario (`install_list_remove`) | No group-tier coverage of skill activation under a gate, install failure/denial, or trusted-vs-installed tool attenuation. Attenuation rules are in `.claude/rules/skills.md`. |
| **Telegram's device-link handshake against real MTProto** is untested at every tier | The handshake *through production wiring* is now covered — `group_device_link/scenario_handshake_mints_and_serves.rs` drives composition's `DeviceLinkFlowDriver` from start to a minted, ownership-pinned credential account and a linked tool call that resolves to it. What no Rust tier can reach is the **vendor** half: the shipped adapter speaks MTProto over a socket with no injectable seam, so QR acceptance, datacenter migration, 2FA, and flood-wait behaviour are exercised only by a scripted adapter. Closing this needs the gated live-smoke protocol (a real account and a human scanning a code), which no PR provisions. |
| **A failed linked session must not reconnect until `link_revision` changes** | Still unimplemented and therefore untestable at any tier: the pool's revision key and the custody revision gate both exist and are now wired in every deployment, but nothing records a session-level *failure* or refuses reconnection until the revision moves. There is no production behavior to assert. |
| **Memory deletion / retention** is uncovered | `group_memory/` covers write, read, search, tree, and binding gating — nothing covers removal, eviction, or the "LLM data is never deleted" invariant from the root `CLAUDE.md`. |
| **Cross-actor isolation for triggers and extensions** is thin at group tier | `group_multiuser/` covers threads, memory, auto-approve and turn state. Extensions get one `with_actor_id` scenario; triggers get none. |
| **Attachments** have no group scenario | Covered flat (`attach.rs`) and in the browser (`test_reborn_webui_v2_legacy_attachments.py`), but not cross-thread/cross-actor. |
| **Sub-agents** have no group scenario | `reborn_subagent_spawn_e2e.rs` + `subagent_await_edge.rs` only. |
| **Python E2E skip/xfail debt** | Tracked separately in `tests/e2e/E2E_DEBT.md` (ClawHub skills, legacy Gmail/MCP OAuth prerequisites, Telegram OAuth placeholders, portfolio widget, v2 auth/OAuth xfails). Do not silently un-xfail. |
| **Live-model tool-misuse patterns** are documented but not gated | `tests/e2e/LIVE_TOOL_FAILURES.md` — the assistant claiming a write it never made, etc. No test fails on these today. |
| **Observability** has no scenario coverage | `crates/substrates/ironclaw_observability` is latency-trace macros over `tracing` only — there is no exporter backend to exercise. |

---

## 8. Adding a scenario

Follow `.claude/rules/testing.md` for tier selection, test-first, and
extend-before-adding — this document does not restate those rules. Once the
test exists:

1. If you added a new scenario rather than extending an existing one, the row
   you write below must explain why an existing scenario couldn't absorb it.
2. Register the binary if it's a new Rust bin (`[[test]]` in the workspace `Cargo.toml`).
3. **Add the row here.** See the maintenance rule at the top.
