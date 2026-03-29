# Development History

Summary of the Claude Code sessions that built the engine v2, self-improvement system, and Python orchestrator. This helps new contributors understand *why* things were designed the way they are.

## Session 1: Engine v2 Foundation (2026-03-20 to 2026-03-22)

Built the core engine crate (`crates/ironclaw_engine/`) from scratch in 6 phases:

- **Phase 1**: Core types (Thread, Step, Capability, MemoryDoc, Project), trait definitions (LlmBackend, Store, EffectExecutor), thread state machine. 32 tests.
- **Phase 2**: Execution engine (Tier 0) — CapabilityRegistry, LeaseManager, PolicyEngine, ThreadManager, ExecutionLoop with structured tool calls. 74 tests.
- **Phase 3**: CodeAct executor (Tier 1) — Monty Python interpreter integration, RLM pattern (context-as-variables, FINAL(), llm_query(), output truncation, Step 0 orientation). 74 tests.
- **Phase 4**: Memory and reflection — RetrievalEngine, reflection pipeline (Summary/Lesson/Issue/Spec/Playbook docs), context compaction, rlm_query() recursive sub-agents, budget controls. 78 tests.
- **Phase 5**: Conversation surface — ConversationManager routing UI messages to threads. 85 tests.
- **Phase 6**: Bridge adapters — LlmBridgeAdapter, EffectBridgeAdapter, HybridStore, EngineRouter. Parallel deployment via `ENGINE_V2=true`. 151 tests.

**Key design decision**: The engine has zero dependency on the main ironclaw crate. All interaction goes through three traits (LlmBackend, Store, EffectExecutor) implemented by bridge adapters.

## Session 2: Debugging via Traces (2026-03-22 to 2026-03-23)

Ran the engine end-to-end with real LLMs and discovered 8 bugs through trace analysis:

1. Tool name hyphens vs underscores (`web-search` vs `web_search`)
2. Double-serialization of JSON tool output
3. UTF-8 byte-index slicing panics on multi-byte characters
4. Code block detection missing in plain completion path
5. Missing system prompt on thread spawn
6. Empty messages sent to LLM
7. `web_fetch` example in prompt (nonexistent tool)
8. False positive `missing_tool_output` trace warning

**Key insight**: Every fix followed the same loop (trace → human reads → human edits Rust → rebuild). This became the motivation for the self-improving engine design.

## Session 3: Mission System (2026-03-24)

Built the Mission system for long-running goals that spawn threads over time:

- `MissionManager` with create/pause/resume/complete lifecycle
- `MissionCadence`: Cron, OnEvent, OnSystemEvent, Webhook, Manual
- `build_meta_prompt()` — assembles mission goal + current focus + approach history + project docs + trigger payload
- `process_mission_outcome()` — extracts next_focus and goal-achieved status from thread responses
- Cron ticker (60s interval)
- 7 E2E mission flow tests

**Key design decision**: Missions evolve their strategy via `current_focus` and `approach_history`. Each thread gets a meta-prompt that includes what was tried before.

## Session 4: Review Fixes + Self-Improvement Foundation (2026-03-25, morning)

Fixed 4 review comments (P1/P2 severity) in the engine v2 bridge:

1. **SSE events scoped to user** — `broadcast_for_user()` instead of `broadcast()`
2. **Per-user pending approvals** — HashMap keyed by user_id instead of global Option
3. **Reset tool-call limit counter** — reset before each thread, not monotonic
4. **Only auto-approve on "always"** — one-off "yes" no longer persists

Then built the self-improvement foundation:

- Runtime prompt overlay via MemoryDoc (prompt builder becomes async + Store-aware)
- `fire_on_system_event()` — wires the previously-unimplemented OnSystemEvent cadence
- `start_event_listener()` — subscribes to thread events, fires matching missions
- `ensure_self_improvement_mission()` — creates the built-in self-improvement Mission
- `process_self_improvement_output()` — saves prompt overlays and fix patterns
- Seed fix pattern database with 8 known patterns

## Session 5: Autoresearch-Inspired Redesign (2026-03-25, afternoon)

Studied [karpathy/autoresearch](https://github.com/karpathy/autoresearch) and redesigned the self-improvement approach:

**Before**: Vague goal prompt, structured JSON output, reactive only.
**After**: Concrete `program.md`-style prompt with exact loop steps, plain text + tool-use (agent uses tools directly like autoresearch), enriched trigger payload with actual error messages.

Key takeaways applied from autoresearch:
- The entire "research org" is a markdown prompt with an explicit loop
- The agent uses tools directly (shell, grep, git) rather than emitting structured output
- Results tracked in a simple append-only log
- "NEVER STOP" — the agent is autonomous within constraints

## Session 6: Python Orchestrator (2026-03-25, evening)

The pivotal architectural change. Motivated by the question: *"What if we move some part of the engine inside CodeAct itself?"*

**The realization**: All the bugs from Session 2 were in the "glue" between the LLM and tools — output formatting, tool dispatch, state management, truncation. These functions are Python-natural. If they were Python, the self-improvement Mission could fix them without a Rust rebuild.

**Research**: Verified that Monty supports nested VM execution (`rlm_query()` already does exactly this — suspends parent VM, runs child ExecutionLoop, resumes parent). No shared state, ~50KB per suspended VM.

**Implementation** (4 commits):

1. **Host function module** (`executor/orchestrator.rs`) — 11 host functions exposed to Python via Monty suspension: `__llm_complete__`, `__execute_code_step__`, `__execute_action__`, `__check_signals__`, `__emit_event__`, `__add_message__`, `__save_checkpoint__`, `__transition_to__`, `__retrieve_docs__`, `__check_budget__`, `__get_actions__`.

2. **Default orchestrator** (`orchestrator/default.py`) — The v0 Python orchestrator that replicates the Rust loop logic. Helper functions (extract_final, format_output, signals_tool_intent) defined before run_loop for Monty scoping.

3. **Switchover** — Replaced the 900-line `ExecutionLoop::run()` with an 80-line bootstrap. Key debugging: Monty's `ExtFunctionResult::NotFound` (not `Error`) for user-defined functions, FINAL result propagation, step_count tracking via `__emit_event__("step_completed")`.

4. **Versioning + rollback** — Failure tracking via MemoryDoc, auto-rollback after 3 consecutive failures, `OrchestratorRollback` event. Self-improvement Mission goal updated with Level 1.5 orchestrator patch instructions.

**Key debugging moment**: The orchestrator's helper functions (`extract_final`, `format_output`) were defined after `run_loop` in the Python file. Monty couldn't find them because the default `FunctionCall` handler returned `ExtFunctionResult::Error` instead of `ExtFunctionResult::NotFound`. The fix: return `NotFound` for unknown functions so Monty falls through to its own namespace resolution. Then move helpers above `run_loop` to avoid any ordering issues.

**Final state**: 189 tests pass, zero clippy warnings. The Python orchestrator is the execution engine. The Rust layer is the kernel.

## Session 7: Integration Scaling Research (2026-03-26)

Studied [Pica](https://github.com/withoneai/pica) (formerly IntegrationOS, 200+ third-party API integrations) to understand how to rapidly scale the number of available integrations in IronClaw.

**Pica's architecture**: Integrations are MongoDB documents, not code. Each platform has a `ConnectionDefinition` (identity + auth schema) and N `ConnectionModelDefinition` records (one per API endpoint: URL, method, auth method, schemas, JS transform functions). A generic executor dispatches requests. OAuth definitions embed JavaScript compute functions executed by a TypeScript service. Adding a new platform = inserting documents, no code changes.

**Analysis of IronClaw v1 tools**: Audited all 37 built-in tools. Only 3 (image_gen, image_analyze, image_edit) are HTTP API wrappers. The other 34 are local computation, filesystem, orchestration, or system management — none convertible to data-driven definitions. The value isn't converting existing tools; it's enabling hundreds of new integrations.

**Key finding — deterministic executors don't solve the LLM problem**: Even with a Pica-style executor, each integration action must be registered as a tool in the LLM's context. At 200+ tools:
- ~20,000 tokens always-on cost (tool definitions sent every request)
- LLM tool selection accuracy degrades beyond ~20-30 tools
- The LLM still constructs parameters and can get them wrong
- Deterministic execution only helps *after* the LLM correctly selects the tool and params

**The realization**: In engine v2, Capabilities already bundle actions + knowledge. For API integrations, a Capability's knowledge text teaches the LLM how to call the platform's API using the generic `http` action. This is superior to dedicated tools because:
- Tool list stays small (just `http` + core actions) — high selection accuracy
- Knowledge loaded on-demand per thread context — zero cost for unused integrations
- ~350 tokens of knowledge covers 4+ API endpoints (the LLM generalizes)
- Adding a new platform = writing markdown knowledge, no Rust code

**Remaining gap**: OAuth token acquisition requires a dedicated `oauth_init` action (LLM can't do redirect flows). Capability knowledge instructs the LLM to call it before using the API.

**Decision**: Use Capabilities as knowledge-bearing integration definitions. Write knowledge text for top 20 platforms. Build one `oauth_init` action. Skip the Pica-style deterministic executor — it solves the wrong problem for LLM agents.

## Session 8: Skills-Based OAuth & Mission Leases (2026-03-27)

Two independent improvements driven by real usage issues.

### Skills-Based Credential System

Studied all OAuth issues reported on GitHub (#1537, #902, #1500, #557, #1441, #1443, #992, #999) and [Pica](https://github.com/withoneai/pica)'s OAuth implementation to design a robust credential system that moves API authentication from WASM modules to skills.

**The problem**: OAuth/credential injection was coupled to WASM `capabilities.json` files. This broke on hosted TEE (#1537), had confusing UX (#902), failed for multi-tool auth (#1500), and lacked user isolation for multi-tenant (#557).

**The insight**: The `skills/github/SKILL.md` already demonstrated the pattern — skill instructs LLM to call `http` tool, credentials auto-injected by host. The gap was that credential declarations lived in WASM, not skills.

**Implementation** (6 files created/modified in `ironclaw_skills`, 4 in main crate):

1. **Credential types in skill frontmatter** — `SkillCredentialSpec`, `SkillCredentialLocation`, `SkillOAuthConfig`, `ProviderRefreshStrategy` in `crates/ironclaw_skills/src/types.rs`. Skills declare credentials in YAML; values never in LLM context.

2. **Validation** — HTTPS enforcement on OAuth URLs, credential name patterns, non-empty hosts. Invalid specs logged and skipped during registration.

3. **Registry bridge** — `credential_spec_to_mapping()` converts skill specs to `CredentialMapping` and registers in `SharedCredentialRegistry`. Wired into `app.rs` after skill discovery.

4. **HTTP tool hardening** — Four security improvements:
   - Block LLM-provided auth headers (`Authorization`, `X-API-Key`) for hosts with registered credentials (prevents prompt injection exfiltration)
   - Structured `authentication_required` error when credentials are missing (guides LLM to `auth_setup`)
   - Strip sensitive response headers (`Set-Cookie`, `WWW-Authenticate`, `Authorization`) before LLM sees them
   - Scan response body through `LeakDetector` to catch APIs echoing back tokens

5. **Pica patterns adopted**: connection testing before persisting, per-provider refresh strategies (`Standard`/`ReauthorizeOnly`/`Custom`), auth header stripping from responses, encryption versioning (forward-looking).

**Test coverage**: 18 type tests + 15 validation tests + 11 conversion/registration tests + 3 HTTP hardening tests + 10 integration tests in `tests/skill_credential_injection.rs`. 315 tests in skills+engine crates, zero clippy warnings.

### Mission Lease Fix

Users reported `"No lease for action 'routine_create'"` when asking the engine to create routines.

**Root cause**: `routine_create` was a v2 mission function handled by `EffectBridgeAdapter::handle_mission_call()`, but `structured.rs` checks capability leases *before* calling the EffectExecutor. Mission functions were never registered as capabilities, so no lease existed.

**Fix**: Registered `mission_create`, `mission_list`, `mission_fire`, `mission_pause`, `mission_resume`, `mission_delete` as a `"missions"` capability in `router.rs`. Descriptions mention "routine" so the LLM maps user intent correctly. Removed all `routine_*` aliases from the effect adapter — `routine_*` names added to `is_v1_only_tool()` blocklist with clear error directing to `mission_*`.

## Session 9: Trace Pipeline Fix, Monty Builtins, Self-Awareness (2026-03-28)

Three fixes driven by analyzing a live engine trace (`engine_trace_20260328T030519.json`) from the hourly Iran-region monitor mission.

### Event Pipeline Loss in CodeAct

**The bug**: The `no_tools_used` trace issue fired as a false positive — the mission thread called `web_search` 5 times, `llm_context` once, and `llm_query` once, yet the trace had zero `ActionExecuted` events.

**Root cause**: `handle_execute_code_step()` in `orchestrator.rs` received `CodeExecutionResult::events` (populated by `dispatch_action()` in `scripting.rs`) but never transferred them to `thread.events` or broadcast them via `event_tx`. The function took `&Thread` (immutable) and had no access to the event broadcast channel. Compare with `handle_execute_action()` which correctly calls `emit_and_record()` for each action.

**Fix**: Changed `handle_execute_code_step()` to take `&mut Thread` + `event_tx`, iterate over `result.events`, push each to `thread.events` and broadcast via `event_tx` — same pattern as `handle_execute_action()`. The `no_tools_used` detector in `trace.rs` now works correctly for CodeAct because `ActionExecuted` events are present.

### globals() NameError in Monty

**The bug**: LLM-generated code used `"mission_create" in globals()` to probe available capabilities before calling them. Monty doesn't implement `globals()` as a builtin, so NameLookup returned `Undefined` → NameError → code execution failure.

**Fix**: Added `globals`/`locals` to the NameLookup handler as callable function stubs, and a FunctionCall handler that returns a `Dict` of all known action names (from capability leases) as keys. Code like `"tool_name" in globals()` now works for capability probing.

### Platform Self-Awareness

**The problem**: The agent had no knowledge of its own identity. It didn't know it was IronClaw, its GitHub repo, its version, active channels, LLM backend, or database. The system prompt just said "You are IronClaw Agent, a secure autonomous assistant" with no specifics.

**The insight**: Identity infrastructure was 85% built — `IDENTITY.md`, `SOUL.md`, `USER.md`, `AGENTS.md` injection worked for *user* identity. But nothing existed for *platform* identity. This isn't workspace-level (it changes with runtime config), so a seed file was wrong — it needed to be injected dynamically.

**Implementation** (8 files):

1. **`PlatformInfo` struct** (`executor/prompt.rs`) — version, llm_backend, model_name, database_backend, active_channels, owner_id, repo_url. `to_prompt_section()` renders a `## Platform` block.

2. **CodeAct path** — `build_codeact_system_prompt()` accepts optional `PlatformInfo`, injects before tool listing.

3. **Tier 0 path** — `Reasoning` struct gets `with_platform_info()` builder, `build_runtime_section()` prepends the platform block.

4. **Runtime wiring** — `Agent::platform_info()` constructs from `AgentDeps` (version from `CARGO_PKG_VERSION`, backend/model/owner from deps, channels from `ChannelManager`).

**Test coverage**: 2 new tests (platform info injection + absence). 195 engine tests pass, zero clippy warnings.

## Session 10: Workspace Restructure, /expected Command, Cleanup (2026-03-28)

Large quality-of-life session focused on making the engine's internal state inspectable and the self-improvement loop actionable.

### Workspace Storage Overhaul

The engine stores all v2 state (threads, missions, knowledge, orchestrator code) in the workspace `memory_documents` table. Previously: 227 files with opaque UUID filenames, JSON-wrapped content, no cleanup, no index.

**New layout** (`src/bridge/store_adapter.rs` — full rewrite):

```
engine/
├── README.md                              (auto-generated index)
├── knowledge/                             (frontmatter+markdown, human-readable)
│   ├── lessons/{slug}--{id8}.md
│   ├── skills/{slug}--{id8}.md
│   └── summaries/{slug}--{id8}.md
├── orchestrator/
│   ├── v0.py                              (compiled-in default, auto-synced)
│   ├── codeact-preamble-overlay.md        (runtime prompt patches)
│   └── failures.json
├── projects/{slug}/
│   ├── project.json
│   └── missions/{slug}/
│       └── mission.json                   (working files can go alongside)
└── .runtime/                              (internal, hidden from browsing)
    ├── threads/active/, threads/archive/
    ├── leases/, events/, steps/, conversations/
```

Key design decisions:
- **Slugified filenames** from titles — `validate-tool-names-before-call--65c9f5cd.md` instead of UUID.json
- **Frontmatter+markdown** for knowledge docs — YAML metadata header + raw content body. `memory_read` returns human-readable markdown, not wrapped JSON
- **Orchestrator + prompts together** — both are self-modifiable runtime code; prompt overlays sit alongside Python versions
- **Missions under projects** — each mission gets a named folder where the self-improvement agent can store working files
- **`.runtime/` prefix** for internal state — threads, leases, events hidden from casual `memory_tree` browsing
- **Terminal state cleanup** — completed/failed threads archived to compact JSON summaries, dead leases deleted. Runs at startup and periodically
- **Auto-generated README** at `engine/README.md` with knowledge doc counts, mission status, active thread count

### /expected Command (User Feedback Loop)

New submission command: `/expected <what should have happened>`. Captures recent conversation turns (last 5: user input, tool calls, responses, errors) and fires a `user_feedback:expected_behavior` system event into the self-improvement pipeline.

**Flow**: User → `/expected should have logged in via GitHub OAuth` → handler packages recent context → fires via both v2 MissionManager and v1 RoutineEngine → expected-behavior learning mission investigates gap, classifies root cause (MISSING_CAPABILITY / WRONG_TOOL_CHOICE / PROMPT_GAP / CONFIG_ISSUE / BUG), applies fix.

Files: `src/agent/submission.rs` (parser), `src/agent/commands.rs` (handler), `src/agent/agent_loop.rs` (mission_manager slot + dispatch), `src/bridge/router.rs` (wiring), `crates/ironclaw_engine/src/runtime/mission.rs` (4th learning mission), `crates/ironclaw_engine/prompts/mission_expected_behavior.md` (goal prompt).

### Other Fixes

- **NeedApproval state transition** — orchestrator Python now calls `__transition_to__("waiting")` before returning approval outcomes. Rust safety net in loop_engine.rs. Previously: denying a tool use errored with "thread not resumable from Running".
- **Monty runtime limitations documented** in CodeAct preamble — no stdlib, single imports, no classes/with/match. Agent no longer tries `import csv, io, math`.
- **`MONTY.md` tracking file** — pin version, all limitations, upgrade process.
- **Gateway new-thread fix** — `createNewThread()` now eagerly resets read-only state.
- **Glob re-exports removed** — `pub use ironclaw_safety::*` and `pub use ironclaw_skills::*` deleted; ~35 files migrated to direct imports.
- **Clippy cleanup** — collapsible ifs, shadow imports, missing SkillActivated match arm, duplicate repl.rs arms. Zero warnings across all crates.
- **Mission prompt templates extracted** to `crates/ironclaw_engine/prompts/mission_*.md` from inline Rust strings.

## Session 11: E2E Test Suite + Engine Hardening (2026-03-28 to 2026-03-29)

Analyzed two production traces (`engine_trace_20260329T011339.json`, `engine_trace_20260329T052431.json`) that revealed 5 silent failures in the v2 engine. Built comprehensive E2E tests that exposed 9 additional engine bugs, all fixed in this session.

### Trace-Driven Fixes (before E2E tests)

1. **Tool result desync on RequireApproval** — `handle_execute_action()` returned without calling `emit_and_record()` for RequireApproval, leaving orphaned `tool_calls` in the OpenAI message history. On thread resume → 400 "No tool output found for function call" → 3 retries → thread failure.

2. **LLM installs WASM tool instead of using skill** — `## Extensions` section told LLM to `tool_search`, competing with `## Active Skills`. Added skill-aware guidance to both v1 `Reasoning` and v2 `default.py` `format_skills()`.

3. **HTTP tool blocks unauthenticated requests** — Credential lookup returned `authentication_required` immediately when secret was missing. Changed to inject-if-available: proceed without auth, only error on 401/403.

4. **NeedAuthentication not wired in CodeAct** — `EffectBridgeAdapter` returned `Ok(ActionResult { is_error: true })` instead of `Err(NeedAuthentication)`. Wired `NeedAuthentication` through scripting.rs `DispatchResult`, orchestrator.rs `handle_execute_action`, default.py, loop_engine.rs safety net, and router.rs NeedAuthentication handler.

5. **CodeAct gives up after one tool error** — Added error recovery section to `codeact_postamble.md`.

### E2E Test Suite (5 files, 12 tests)

Built a v2-engine-specific E2E test framework: mock API servers (aiohttp), mock LLM tool call patterns, dedicated ironclaw server fixtures with `ENGINE_V2=true`, `HTTP_ALLOW_LOCALHOST=true`, `SECRETS_MASTER_KEY`.

| File | Tests | Coverage |
|------|-------|---------|
| `test_v2_engine_auth_flow.py` | 4 | Skill activation, NeedAuthentication → auth prompt → token → retry → mock API receives token, credential persistence |
| `test_v2_engine_auth_cancel.py` | 2 | Cancel during auth prompt, server responsive after cancel |
| `test_v2_engine_approval_flow.py` | 4 | Approve yes/no/always (text-based), approval prompt mentions tool name |
| `test_v2_engine_error_handling.py` | 2 | Max iterations (30 step limit), tool intent nudge recovery |

### Bugs Found and Fixed by Running E2E Tests

6. **`HTTP_ALLOW_LOCALHOST` flag** — HTTP tool's SSRF protection blocked `http://127.0.0.1`, making mock-server-based testing impossible. Added `OnceLock`-backed env var check.

7. **`EngineError::NeedApproval`** — Effect adapter returned `LeaseDenied` for tools needing approval (not auto-approved, no credential backing). Engine treated it as generic error → thread failed. Added `NeedApproval` variant and wired through orchestrator and scripting dispatch.

8. **v1 DB write for non-Completed outcomes** — `await_thread_outcome` only wrote to v1 DB for `Completed { response }`. NeedApproval/NeedAuthentication responses were invisible in history API → `state=Failed`. Moved write to cover all outcomes.

9. **`pending_approval` thread ID mismatch** — History endpoint passed v1 session UUID as hint to engine pending approval lookup (different UUID space). Cache miss every time. Removed hint.

10. **`SECRETS_MASTER_KEY` required** — Without it, `init_secrets()` returns early → no SecretsStore → HttpTool has no credential injection → NeedAuthentication never triggers. Test fixtures now set the key.

11. **`user_id: "orchestrator"` hardcoded** — `ThreadExecutionContext` used `"orchestrator"` for secrets lookup, but credentials stored under real user_id. Changed to read from `thread.metadata["user_id"]`.

12. **`host_matches_pattern` port matching** — Skill hosts like `"127.0.0.1:8080"` didn't match `host_str()` output `"127.0.0.1"`. Added port-stripping logic.

13. **Cancel during auth stored message as credential** — `SubmissionParser` parsed `"cancel"` as `ApprovalResponse { approved: false }`, bypassing `handle_with_engine`'s PendingAuth check. Next `UserInput` message was treated as token and stored. Added `has_pending_auth()` check to route approval-like submissions through `handle_with_engine` when auth is pending.

14. **Cancel doesn't stop engine thread** — Added `engine_thread_id` to `PendingAuth`, call `stop_thread` on cancel. Also added v1 DB write for cancel response.

### Infrastructure

- **`mock_llm.py`** — Added runtime-configurable `_github_api_url` via `POST /__mock/set_github_api_url`, tool call patterns for `list.*issues`, `loop forever`, `list.*drive.*files`, canned responses for tool intent nudge.
- **Mock API servers** — Per-test aiohttp servers with strict Bearer token validation (`ghp_*` prefix), token tracking, reset endpoints.
- **`HTTP_ALLOW_LOCALHOST`** — New env var flag that relaxes HTTPS-only and SSRF checks for `http://` and `127.0.0.1` targets. For E2E testing only.

## Architecture Evolution

```
Session 1-2:  Rust loop (900 lines) → works but bugs in glue layer
Session 3:    + Missions (long-running goals, evolving strategy)
Session 4:    + Self-improvement Mission (fires on issues, fixes prompts)
Session 5:    + Autoresearch-style goal prompt (concrete, not vague)
Session 6:    Rust loop → Python orchestrator (self-modifiable)
              900 lines Rust → 80 lines Rust bootstrap + 230 lines Python
Session 7:    Integration scaling: Capabilities as knowledge → http action
              (not Pica-style per-action tools — tool list bloat kills LLM accuracy)
Session 8:    Skills-based OAuth (credential specs in YAML frontmatter)
              + HTTP tool zero-leak hardening + mission capability leases
Session 9:    CodeAct event pipeline fix (ActionExecuted events were lost)
              + Monty globals() builtin + platform self-awareness injection
Session 10:   Workspace restructure (human-readable paths, frontmatter,
              cleanup, README) + /expected feedback loop + approval fix
Session 11:   E2E test suite (12 tests across 5 files) → found 9 engine bugs
              + HTTP_ALLOW_LOCALHOST + NeedApproval/NeedAuthentication wiring
              + user_id fix + cancel routing fix + host_matches_pattern fix
Session 12:   Kernel-level auth — pre-flight credential gate, post-install
              auth pipeline, tool_auth/tool_activate removed from v2 LLM,
              AuthManager centralizes credential checks + setup instructions
Session 13:   Plan mode — autonomous long-running tasks via composing
              existing v2 primitives (MemoryDoc, Mission, SSE events)
              + /plan command + plan-mode skill + live checklist UI
```

## Session 12: Kernel-Level Authentication (2026-03-29)

Reworked authentication from a reactive LLM-driven flow to a proactive kernel-level interrupt, based on the design doc in `rework-auth.md`.

### Problem

Auth was a 3-step non-deterministic chain: tool fails with 401 → LLM "decides" to call `tool_auth` → LLM "decides" to retry. Each decision was a coin flip, giving ~50-70% success rate on a flow that should be 100%.

### Solution: Pre-flight Auth Gate

New `AuthManager` (`src/bridge/auth_manager.rs`) centralizes credential checking. The `EffectBridgeAdapter` now checks credentials BEFORE executing tool calls:

```
LLM calls http(url="https://api.github.com/...") →
  Pre-flight: extract host → SharedCredentialRegistry.find_for_host() →
    Secret exists? → execute normally
    Secret missing? → NeedAuthentication (tool never executes, no 401)
```

### Key Decisions

1. **Defense in depth, not replacement**: The pre-flight gate is the primary path, but the existing reactive 401 detection and text-based `authentication_required` fallback are kept. Removing fallbacks would create a dead-end where the LLM asks for credentials but the kernel doesn't recognize the auth state.

2. **tool_auth/tool_activate removed from v2 LLM context**: These are now kernel-internal. The LLM never sees them in its tool list and gets an error if it somehow calls them. Auth is fully automatic from the LLM's perspective.

3. **Post-install auth pipeline**: After `tool_install` succeeds, the kernel auto-checks `ExtensionManager::check_tool_auth_status_pub()` and either auto-activates (Ready), initiates auth flow (NeedsAuth), or appends setup instructions (NeedsSetup). The LLM doesn't need to call `tool_auth` → `tool_activate` manually.

4. **NeedsAuth tools stay visible, NeedsSetup tools hidden**: Tools that need OAuth/tokens stay in the LLM's tool list so it can attempt to use them (triggering the pre-flight gate which starts the auth flow). Tools that need admin setup (client_id/secret) are hidden since they can't be resolved in chat.

5. **Setup instruction deduplication**: The skill-registry lookup for credential setup instructions was duplicated in 3 places in `router.rs`. Now centralized in `AuthManager::get_setup_instructions()`.

### Files Changed

| File | Role |
|------|------|
| `src/bridge/auth_manager.rs` | **New** — AuthManager, AuthCheckResult, ToolReadiness, credential checking, 8 unit tests |
| `src/bridge/effect_adapter.rs` | Pre-flight gate, post-install pipeline, v1 auth tool blocking + filtering |
| `src/bridge/router.rs` | AuthManager wired in init_engine(), deduplicated setup lookups, text fallback tracing |
| `src/extensions/manager.rs` | Public wrapper for check_tool_auth_status() |
| `tests/e2e/scenarios/test_v2_kernel_auth_preflight.py` | 5 E2E tests: preflight, retry, persistence, tools hidden, cancel |

### What's NOT Changed

- Engine crate (`crates/ironclaw_engine/`) — `NeedAuthentication` already existed
- HTTP tool (`src/tools/builtin/http.rs`) — existing 401 detection stays as safety net
- WASM credential injection — zero-exposure model unchanged
- `/api/chat/auth-token` endpoint — already bypasses LLM correctly

## Session 13: Plan Mode — Autonomous Long-Running Tasks (2026-03-29)

Designed and built plan mode for autonomous task execution, inspired by OpenAI Codex's `update_plan` checklist and Claude Code's file-based plan mode. The key insight: both systems enforce plan mode restrictions entirely through prompting, not tool removal. IronClaw's implementation composes existing v2 primitives (MemoryDoc, Mission, SSE events) with a skill and thin command shim.

### Research

Studied two external references:
- **OpenAI Codex** (`github.com/openai/codex`) — Three collaboration modes (Plan/Default/Execute), `update_plan` tool for structured checklist rendering, `<proposed_plan>` tag parsing. Plan restrictions are prompt-based, not tool-level.
- **Claude Code plan mode** ([lucumr.pocoo.org](https://lucumr.pocoo.org/2025/12/17/what-is-plan-mode/)) — Plans written to filesystem as markdown. Phased approach (Understand/Design/Review/Final). `ExitPlanMode` tool signals completion. All enforcement is prompt-based.

### Design: Compose, Don't Build

Rather than adding engine states or new worker modes, plan mode maps to existing v2 primitives:

| Concept | V2 Primitive |
|---------|-------------|
| Plan document | `MemoryDoc` with `DocType::Plan` |
| Execution | `Mission` (Manual cadence) → spawns `ThreadType::Mission` threads |
| Progress | `AppEvent::PlanUpdate` SSE event → live UI checklist |
| Learning | Existing learning missions (auto-fire after thread completion) |
| Behavior | `skills/plan-mode/SKILL.md` (prompt engineering) |

### Implementation

1. **`DocType::Plan`** — Added to engine's `MemoryDoc` enum. Plans are project-scoped, retrievable via `RetrievalEngine`, and injected into mission threads by `build_meta_prompt()`.

2. **`AppEvent::PlanUpdate`** — New SSE event carrying a full checklist snapshot (`PlanStepDto` with index, title, status, result). Modeled after Codex's `update_plan` — always sends the full list (not diffs) so the UI is idempotent.

3. **`plan_update` tool** (`src/tools/builtin/plan.rs`) — Like Codex's `update_plan`, "this function doesn't do anything useful — it gives the model a structured way to record its plan that clients can render." Broadcasts `PlanUpdate` SSE event. Registered in `register_builtin_tools()` without SSE, then upgraded with SSE manager in `main.rs` post-gateway-init.

4. **`/plan` command** — `PlanSubcommand` enum (Create/Approve/Status/Revise/List) parsed in `submission.rs`. All subcommands rewrite to `Submission::UserInput` with `[PLAN MODE]` prefix, which activates the plan-mode skill. No new handler methods — the LLM + skill use existing tools (`memory_write`, `mission_create`, `mission_fire`, `plan_update`).

5. **Plan-mode skill** (`skills/plan-mode/SKILL.md`) — Trusted skill activated on `[PLAN MODE]` keyword. Defines the full protocol: plan document format (markdown with checkboxes), creation flow (search context → write plan → emit checklist), approval flow (create mission → fire → track), execution protocol (update steps, handle failures), and revision flow.

6. **Web UI** — `plan_update` SSE listener + `renderPlanChecklist()` in `app.js`. Inline chat widget with status badge, step checklist (checkmarks/spinners/circles), and progress summary. CSS reuses existing activity card patterns.

7. **E2E tests** — 5 scenarios in `test_plan_mode.py`: create renders checklist, approve changes status, status shows progress, list via API, command parsing. Mock LLM patterns return `plan_update` tool calls for plan-related messages.

### Key Design Decisions

1. **`/plan` rewrites to `UserInput`, not a new handler** — The skill handles all logic using existing tools. The command is pure UX sugar. This means zero new methods in `commands.rs` (only help text).

2. **Full checklist snapshots, not diffs** — Each `PlanUpdate` SSE event carries the entire step list. The UI replaces the DOM on every event. This avoids client-side state synchronization bugs.

3. **Plan execution via Mission, not Job** — Missions track `current_focus` (which step to work on next), `approach_history` (what was tried), and `thread_history` (all execution threads). The outcome watcher updates these automatically. Jobs don't have this evolving-strategy primitive.

4. **No tool restriction in plan mode** — Unlike Codex which prompts the LLM not to mutate, IronClaw's plan-mode skill naturally guides the LLM to only use read/search/write tools during planning. During execution (mission thread), all tools are available per capability leases.

### Files Changed

| File | Role |
|------|------|
| `crates/ironclaw_engine/src/types/memory.rs` | `DocType::Plan` variant |
| `crates/ironclaw_common/src/event.rs` | `PlanStepDto`, `AppEvent::PlanUpdate`, tests |
| `src/tools/builtin/plan.rs` | **New** — `PlanUpdateTool` (SSE broadcast) |
| `src/tools/builtin/mod.rs` | Module + export registration |
| `src/tools/registry.rs` | `register_plan_tools()`, auto-register in builtins |
| `src/main.rs` | Wire SSE into plan tool post-gateway-init |
| `src/tools/schema_validator.rs` | Added to schema validation test |
| `src/agent/submission.rs` | `PlanSubcommand` enum, `Submission::Plan`, parsing |
| `src/agent/agent_loop.rs` | `Submission::Plan` match arm (rewrite to UserInput) |
| `src/agent/commands.rs` | Help text for /plan commands |
| `skills/plan-mode/SKILL.md` | **New** — Full plan protocol skill |
| `src/channels/web/static/app.js` | SSE listener + `renderPlanChecklist()` |
| `src/channels/web/static/style.css` | Plan checklist component styles |
| `tests/e2e/mock_llm.py` | Plan mode tool call patterns |
| `tests/e2e/helpers.py` | Plan DOM selectors |
| `tests/e2e/scenarios/test_plan_mode.py` | **New** — 5 E2E tests |

## Key Commits

| Commit | Description |
|--------|-------------|
| `8be19a4` | Phase 1: Foundation types + traits |
| `bf7dfb8` | Phase 2: Tier 0 execution engine |
| `b59a0b9` | Phase 3: CodeAct (Monty + RLM) |
| `4bc7ffd` | Phase 4: Memory + reflection + budgets |
| `0827235` | Phase 5: Conversation surface |
| `ac4ced0` | Phase 6: Bridge adapters (parallel deploy) |
| `8180a417` | Self-improving engine via Mission system |
| `cfe856da` | Python orchestrator module + host functions |
| `63756039` | Switch ExecutionLoop to Python orchestrator |
| `080317aa` | All 177 tests pass with orchestrator |
| `46fd2b5d` | Versioning, auto-rollback, 189 tests |
| `606d6571` | Kernel-level pre-flight auth gate for engine v2 |
