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

The counts in each section header are checked-in facts, not estimates. Update them.

---

## 1. Where scenarios live (tier map)

| Tier | Location | What is real | What is faked | Cost |
|---|---|---|---|---|
| **Group scenarios** (§3) | `tests/integration/group_*/scenario_*.rs` | whole Reborn turn, shared runtime + shared stores across several threads | the model, at the vendor-SDK seam | fast, offline, no setup |
| **Flat integration** (§4) | `tests/integration/*.rs`, `tests/integration/auth/*.rs` | whole Reborn turn, one thread; one dedicated sandbox test also uses a real local Docker worker | the model | usually fast/offline; sandbox worker test runs in its Docker CI lane |
| **Binary / parity / QA-trace** (§5) | `tests/*.rs` | composed runtime or shipping binary; recorded real-LLM traces | model responses replayed from committed traces | medium |
| **Python E2E** (§6) | `tests/e2e/scenarios/test_*.py` | current Reborn scenarios use real `ironclaw serve`, HTTP, and Playwright; retained legacy-fixture scenarios are pending migration | LLM (mock or recorded), external SaaS (Emulate/fakes) | slow |

Authoring guides: `tests/integration/CLAUDE.md` (Rust harness, group mechanics,
assertions) and `tests/e2e/CLAUDE.md` (pytest fixtures, Playwright, mock LLM).
Tier-selection rule: `.claude/rules/testing.md`.

## 2. Coverage at a glance

| Area | Group | Flat int | Binary/QA | Python E2E |
|---|---|---|---|---|
| Approvals & permission gates | 10 | ✓ | ✓ | ✓ |
| Auth / credentials / OAuth | 3 | 7 | ✓ | ✓ (heaviest) |
| Extension lifecycle | 14 | 6 | ✓ | ✓ |
| Channels (Slack/Telegram/webhook) | 2 | 3 | ✓ | ✓ |
| Triggers / automations / routines | 10 | 2 | ✓ | ✓ |
| Memory & workspace | 5 | 2 | — | ✓ |
| Skills | 1 | 1 | — | ✓ |
| Multi-user / scope isolation | 5 | 2 | 9 | ✓ |
| Tools & tool dispatch | — | 11 | ✓ | ✓ |
| Turn lifecycle (cancel/steer/retry/restart) | — | 8 | ✓ | ✓ |
| WebUI surfaces & APIs | 2 | 2 | — | ✓ (largest) |
| Durability & restart | 4 | 5 | ✓ | ✓ |
| Security & redaction | — | 3 | ✓ | ✓ |
| Providers (Google/Slack/GitHub contracts) | — | — | ✓ | ✓ |
| Coverage/meta gates | — | 2 | ✓ | ✓ |

Totals: **51** group scenarios · **55** flat integration bins (49 in
`tests/integration/`, 6 in `tests/integration/auth/`) · **39** top-level Rust bins ·
**102** Python scenario files (**868** test functions).

---

## 3. Group scenarios — `tests/integration/group_*/` (51)

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

### 3.4 Memory — `group_memory/` (5)

| The user can… | Evidence |
|---|---|
| Have the assistant write a note in one chat and read it back in another | `scenario_write_then_read_cross_thread.rs` |
| Search memory from a different conversation and find what was written | `scenario_memory_search_finds_seeded.rs` |
| See the real folder structure of their memory | `scenario_memory_tree_reflects_structure.rs` |
| Run a build with memory disabled and have the assistant not even see memory tools | `scenario_disabled_binding_offers_no_memory_tools.rs` |
| Trust that only the memory hooks the provider declares actually fire | `scenario_lifecycle_gates_host_memory_calls.rs` |

### 3.5 Multi-user — `group_multiuser/` (5)

| The user can… | Evidence |
|---|---|
| Share one deployment with someone else and each keep their own threads | `scenario_two_actors_own_threads.rs` |
| Not read another user's memories (and still read their own) | `scenario_memory_isolation_across_actors.rs` |
| Not have another user's "always allow" apply to them | `scenario_auto_approve_isolation_across_actors.rs` |
| Not see another user's run/turn state | `scenario_turn_state_isolation_across_actors.rs` |
| On a per-caller-scoped (served) deployment: read their brand-new workspace as empty instead of an error, approve a gated write that lands in their own `tenants/{tenant}/users/{user}` subtree (never the shared root), and read it back — while a missing sub-path stays a hard error | `scenario_scoped_workspace_isolation.rs` |

### 3.6 Skills — `group_skills/` (1)

| The user can… | Evidence |
|---|---|
| List, install, and remove a skill, with each step visible from a different conversation | `scenario_install_list_remove.rs` |

### 3.7 Triggers & automations — `group_triggers/` (10)

| The user can… | Evidence |
|---|---|
| Create a scheduled automation, then pause/resume/remove it from another conversation | `scenario_verbs_lifecycle.rs` |
| Restart the service and still have their automation | `scenario_trigger_persists_after_reopen.rs` |
| Have a scheduled run ask for approval mid-fire and resume after they answer | `scenario_triggered_gate.rs` |
| Have a scheduled run chain through *two* approval prompts and still be recognised as a scheduled run | `scenario_triggered_chained_gate.rs` |
| See "waiting on you" on an automation whose run is parked, and see it clear when the run ends | `scenario_triggered_gate_hold_visible.rs` |
| Trust that a scheduled run can't create/remove/pause its own automations | `scenario_trigger_self_create_denied.rs` |
| Create an automation from Slack/Telegram and have the result routed back to that conversation | `scenario_external_source_trigger_captures_delivery.rs` |
| Not create an automation pointed at a delivery target this deployment can't reach | `scenario_delivery_target_fail_closed.rs` |
| See and rename their automations in the WebUI, backed by the real trigger store | `scenario_webui_automations_list.rs`, `scenario_webui_automations_rename.rs` |

---

## 4. Flat integration bins — `tests/integration/*.rs` and `tests/integration/auth/*.rs` (55)

One thread, whole real turn. Grouped by what the user experiences.

**Chat & turn lifecycle**
| Behavior | Evidence |
|---|---|
| A plain message gets a persisted reply through the whole real stack | `greeting.rs` |
| Stopping a running turn actually stops it (Cancelled, not Completed) | `cancel.rs` |
| Typing again while the assistant is working queues the message and it gets picked up mid-run | `steering.rs` |
| A flaky model provider is retried and recovered from, with typed errors | `model_recovery.rs` |
| The exact prompt + tool surface sent to the model is snapshot-pinned per iteration | `golden_payload.rs` |
| Approaching the run limit surfaces a recoverable warning | `terminal_warning.rs` |
| Repeating the same inbound message does not start a second run | `idempotent_replay.rs` |
| Spend accounting fires on a real turn | `budget.rs` |
| Sub-agents spawn and awaiting them behaves at the edges | `subagent_await_edge.rs` |

**Tools**
| Behavior | Evidence |
|---|---|
| An HTTP tool call reaches the real egress boundary and the result reaches the model | `tool_call.rs`, `http_matcher.rs` |
| Saved, transcript-shaped JSON can be queried through scoped storage with plain or `$`-rooted paths, and invalid JSON produces model-visible correction guidance | `tool_call.rs` |
| Shell commands dispatch through the real path without spawning an OS process | `process_port.rs` |
| A sandbox-profile shell turn executes as an unprivileged user in a real Docker worker and keeps its workspace across calls | `reborn_sandbox_shell_turn.rs` |
| MCP tools work over a real loopback HTTP MCP server | `mcp.rs` |
| User-registered hosted MCP servers register, authenticate, restore, and invoke | `hosted_mcp_registration.rs` |
| Web search/fetch runs the real Exa MCP handshake | `web_access.rs` |
| Outbound HTTP crosses the real security pipeline (network policy + leak scan) | `real_egress_pipeline.rs` |
| Tools marked host-internal are never advertised to the model, and calls to them are rejected | `extension_visibility.rs`, `surface_disclosure.rs` |
| Bridged tool disclosure mode reaches production's decorator wiring | `tool_disclosure.rs` |
| A capability whose lease expires mid-dispatch does not wedge the run | `lease_wedge.rs` |
| Attachments the user uploads are read back byte-for-byte by the model | `attach.rs` |
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
| Tenant-admin configuration and per-user install/remove stay separate state machines | `extension_user_lifecycle_isolation.rs` |
| The model sees channel setup guidance but not UI-only chrome | `channel_connection_projection.rs` |
| Delivery preferences / connected channels render into the model prompt | `comm_context.rs` |

**Durability, storage & restart**
| Behavior | Evidence |
|---|---|
| Behavior is identical on in-memory and libSQL storage | `backend_matrix.rs` |
| Installed extensions survive a fresh store reopen | `durable.rs` |
| Secrets survive a genuine on-disk reopen | `secrets.rs` |
| Outbound preferences survive a process-level reopen | `outbound_store_durability.rs` |
| Restart sequences over a gated run recover correctly | `generated_restart_sequences.rs` |
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
| Identity resolution runs on the coverage lane | `identity_resolution_smoke.rs` |

One of the 55 registered bins, `delivery_user_journeys.rs`, is a retired Cargo
target with no scenario behavior. Current delivery evidence is listed under
Extensions & channels and the group trigger scenarios; the retired target is
not coverage.

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
| any of the above, replayed from committed real-LLM traces | `reborn_qa_recorded_behavior.rs` (25) |
| a broad smoke set of whole turns on a full runtime | `reborn_qa_smoke_scenarios_e2e.rs` (10) |

**Binary-level behavior**
| Behavior | Evidence |
|---|---|
| A provider failure yields a sanitized, *retryable* failure and the user can retry and resume | `reborn_failure_retry_resume_e2e.rs` (6) |
| Sub-agents spawn end-to-end | `reborn_subagent_spawn_e2e.rs` (5) |
| The shipped Docker image has a usable runtime home | `dockerfile_runtime_home.rs` (19) |
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
`reborn_trace_first_party_tool_coverage.rs` (10),
`reborn_recorded_trace_parity.rs`, `reborn_minimal_dispatch_parity.rs`,
`reborn_response_order_parity.rs`, `reborn_tool_param_coercion_parity.rs`,
`reborn_approval_traces_parity.rs`, `reborn_turn_state_lock_free_submit_parity.rs`.

**Policy & format**: `e2e_trace_runtime_policy_org_ceiling_yolo.rs` (org policy
ceiling × yolo narrowing), `e2e_trace_runtime_policy_serde.rs` (wire-stable policy
enums), `trace_format.rs`, `trace_llm_tests.rs`,
`reborn_coverage_lane_stack_headroom.rs` (CI job must declare stack headroom).

---

## 6. Python E2E scenarios — `tests/e2e/scenarios/` (102 files, 868 tests)

This is an exhaustive inventory, not a claim that every retained scenario is
currently executable. Current Reborn coverage starts `ironclaw serve` through the
`reborn_v2_*` fixtures or exercises a current provider/API harness. Any cited test
that still uses `ironclaw_binary`, `ironclaw_server`, or the legacy `page`/`SEL`
fixtures is inventory-only, non-functional, and pending migration under #6369 —
even when it appears beside current Reborn evidence in the same row. See
`tests/e2e/CLAUDE.md` §"Reborn E2E coverage gate" before adding coverage-manifest
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
| Collapse the sidebar, pick a theme, pick a language — and have it persist | `test_reborn_webui_v2_smoke.py` |
| Reconnect SSE without gaps or duplicates; multiple tabs both get the reply; excess connections are rate-limited | `test_reborn_webui_v2_legacy_sse_history.py`, `test_reborn_webui_v2_streaming_run_control_api.py` (10) |
| Keep execution-only engine threads out of the chat sidebar while preserving deep-linked history | `test_v2_thread_visibility.py` (2; pending legacy migration #6369) |

### 6.2 Tools, approvals & auth prompts (UI)
| The user can… | Evidence |
|---|---|
| Run built-in tools, parallel calls, and multi-step chains and see the result | `test_reborn_webui_v2_legacy_tool_execution.py` (9), `test_v2_engine_tool_lifecycle.py` (6), `test_tool_execution.py` |
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
| Sign in with Google-shaped SSO and have two users keep separate threads | `test_reborn_webui_v2_sso.py` |

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
| Create an automation through chat; rename, pause, resume, reload, and delete it through the UI with persisted API state; filter automations, retry failed runs, and dismiss error toasts | `test_reborn_webui_v2_smoke.py::test_reborn_v2_automation_lifecycle_persists_from_ui`, other automation tests in that file, `test_reborn_webui_v2_automation_trace_outbound_api.py` (4) |
| Create event-triggered routines, have them fire on match, respect cooldown, pause/resume | `test_routine_event_batch.py` (8) |
| Run a full-job routine end-to-end with tools, trigger it manually, see failures in the UI | `test_routine_full_job.py` (3) |
| Have routines run with injected OAuth credentials | `test_routine_oauth_credential_injection.py` (3) |
| Create a Gmail-draft mission, pause it for authentication, and resume it after OAuth | `test_mission_gmail_3133.py` (pending legacy migration #6369) |
| Still reach their v1 routines after upgrading to v2 (#2982) | `test_routines_tab_after_v2_upgrade.py` (11) |
| Use the Missions tab instead of the removed Routines tab and activity strip | `test_v2_activity_shell.py` (2; pending legacy migration #6369) |
| See routines created in one surface from another (owner scope) | `test_owner_scope.py` (3) |
| Browse projects, create one, open a scoped chat, list and download workspace files | `test_reborn_webui_v2_legacy_projects.py` (6), `test_project_detail.py` (3), `test_reborn_v2_file_download.py` (4) |
| Get notified about automation activity and open the thread from the notification | `test_reborn_webui_v2_notifications.py` (4) |

### 6.7 Settings, skills & admin
| The user can… | Evidence |
|---|---|
| Search across settings sections and clear the search | `test_reborn_webui_v2_legacy_settings_search.py` (6), `test_settings_search.py` (5) |
| Add, test, activate, edit and delete a custom inference provider | `test_reborn_webui_v2_legacy_settings_search.py` |
| Add/edit/delete skills, with read-only sources locked | `test_reborn_webui_v2_legacy_skills.py` (3), `test_reborn_webui_v2_skills_api.py` (3), `test_portfolio.py` (10) |
| Use plan mode (`/plan`, checklist, approve, status) | `test_plan_mode.py` (5) |
| As an admin: create users, hand out one-time tokens, page the user list, set roles, suspend/activate, manage write-only secrets, delete users | `test_admin_api.py` (18) |
| Bootstrap as the single-tenant owner with stable identity | `test_ownership_model.py` (8), `test_multi_tenant_greeting.py` |
| Customize their gateway layout/widgets by chatting | `test_widget_customization.py` (4) |

### 6.8 APIs (OpenAI-compatible, filesystem, operator)
| The user can… | Evidence |
|---|---|
| Use the Responses API (non-streaming, streaming SSE, continue, retrieve, context injection, auth/validation errors) | `test_reborn_responses_api.py` (14), `test_reborn_webui_v2_legacy_responses_api.py` (14), `test_responses_api.py` (9) |
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
| Upload attachments within count/size/type limits, with extraction and placeholders | `test_reborn_webui_v2_legacy_attachments.py` (7) |
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
| **Telegram** has no group-tier lifecycle scenario | Slack has `scenario_slack_channel_lifecycle_state_machine.rs`; Telegram's setup resolves through a pairing mechanism the bare group harness doesn't mount (see `scenario_extension_install_github_normal_gate.rs`'s module doc). Telegram is covered at the Python tier only. |
| **Memory deletion / retention** is uncovered | `group_memory/` covers write, read, search, tree, and binding gating — nothing covers removal, eviction, or the "LLM data is never deleted" invariant from the root `CLAUDE.md`. |
| **Cross-actor isolation for triggers and extensions** is thin at group tier | `group_multiuser/` covers threads, memory, auto-approve and turn state. Extensions get one `with_actor_id` scenario; triggers get none. |
| **Attachments** have no group scenario | Covered flat (`attach.rs`) and in the browser (`test_reborn_webui_v2_legacy_attachments.py`), but not cross-thread/cross-actor. |
| **Sub-agents** have no group scenario | `reborn_subagent_spawn_e2e.rs` + `subagent_await_edge.rs` only. |
| **Python E2E skip/xfail debt** | Tracked separately in `tests/e2e/E2E_DEBT.md` (ClawHub skills, legacy Gmail/MCP OAuth prerequisites, Telegram OAuth placeholders, portfolio widget, v2 auth/OAuth xfails). Do not silently un-xfail. |
| **Live-model tool-misuse patterns** are documented but not gated | `tests/e2e/LIVE_TOOL_FAILURES.md` — the assistant claiming a write it never made, etc. No test fails on these today. |
| **Observability** has no scenario coverage | Only `log` and `noop` backends ship (root `CLAUDE.md` → Current Limitations). |

---

## 8. Adding a scenario

1. Pick the tier from §1 and `.claude/rules/testing.md`. Integration-first: production-wired
   Reborn behavior ships with a test in `tests/integration/`.
2. **Extend an existing scenario before writing a new one.** If you add one, the row you
   write in this document must explain why an existing scenario couldn't absorb it.
3. Write it test-first — watch it fail for the right reason.
4. Register the binary if it's a new Rust bin (`[[test]]` in the workspace `Cargo.toml`).
5. **Add the row here.** See the maintenance rule at the top.
