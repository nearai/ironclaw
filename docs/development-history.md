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
```

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
