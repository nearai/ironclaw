# IronClaw Engine v2: Unified Thread-Capability-CodeAct Architecture

**Date:** 2026-03-20
**Updated:** 2026-03-22
**Status:** In Progress (Phases 1-3 complete)
**Goal:** Replace IronClaw's ~10 fragmented abstractions with a unified execution model built on 5 primitives: Thread, Step, Capability, MemoryDoc, Project. Developed as a standalone crate (`ironclaw_engine`) that can be swapped in when it passes all acceptance tests.

---

## Motivation

IronClaw currently has Session, Job, Routine, Channel, Tool, Skill, Hook, Observer, Extension, and LoopDelegate as separate abstractions. All share common patterns (lifecycle, messaging, state, capabilities) but are implemented independently. This causes:

- Duplicated logic across ChatDelegate, JobDelegate, ContainerDelegate
- Inconsistent state machines (SessionState vs JobState vs RoutineState)
- Three separate permission systems (ApprovalRequirement, ApprovalContext, SkillTrust)
- No structured learning from completed work
- No project-level context scoping (all memory in one flat namespace)
- The agentic loop can only do one tool call per LLM turn (no control flow)

## Design Principles

1. **Conversation is not execution** — UI surfaces (chat) are separate from work units (threads)
2. **Everything is a thread** — conversations, jobs, sub-agents, routines are all threads with different types
3. **Capabilities unify tools + skills + hooks** — one install gives you actions, knowledge, and policies
4. **Effects, not commands** — capabilities declare their effect types; a deterministic policy engine enforces boundaries
5. **Memory is docs, not logs** — durable knowledge is structured (summaries, lessons, playbooks), not raw history
6. **CodeAct for capable models** — LLMs write code that composes tools, queries history, and spawns threads
7. **Context as variable, not attention input** (RLM pattern) — thread context is a Python variable in the REPL, not tokens in the LLM window. The model writes code to selectively access it, avoiding context rot on long inputs
8. **Recursive subagent spawning** (RLM pattern) — code can call `llm_query()` to spawn child threads inline. Results are stored as variables, not injected into the parent's context window
9. **Event sourcing from day one** — every thread records a complete execution trace for replay/debugging/reflection

## Key Influences

- **RLM paper** (arXiv:2512.24601, Zhang/Kraska/Khattab, MIT) — context as variable, FINAL() termination, recursive sub-calls, output truncation, compaction
- **Official RLM impl** (alexzhang13/rlm) — 30 max iterations, 20K char truncation, compaction at 85% context, scaffold restoration, FINAL_VAR regex fallback, consecutive error counting, budget/timeout/token limits with inheritance to child RLMs
- **fast-rlm** (avbiswas/fast-rlm) — Step 0 orientation preamble, parallel `asyncio.gather` sub-calls, dual model routing (stronger root, cheaper sub), dual system prompts (leaf vs non-leaf), 2K char truncation (aggressive but fast), fresh runtime per sub-agent
- **Prime Intellect** (verifiers/RLMEnv) — answer dictionary pattern (`{"content": "", "ready": True}`), tools restricted to sub-LLMs only, `llm_batch()` for parallel dispatch, 8K char truncation, FIFO-based sandbox communication, per-REPL-call 120s timeout
- **rlm-rs** (zircote/rlm-rs) — Rust CLI using pass-by-reference chunk IDs, tree-sitter code-aware chunking, hybrid BGE-M3+BM25 search with RRF, SQLite persistence
- **Google ADK RLM** — lazy Path objects (data stays on disk/GCS until code accesses it), massive parallelism with global concurrency limits

## The Five Primitives

| Primitive | Purpose | Replaces |
|-----------|---------|----------|
| **Thread** | Unit of work with lifecycle, parent-child tree, capability leases | Session + Job + Routine + Sub-agent |
| **Step** | Unit of execution (one LLM call + its tool/code executions) | Agentic loop iteration + tool calls |
| **Capability** | Unit of effect (actions + knowledge + policies) | Tool + Skill + Hook + Extension |
| **MemoryDoc** | Unit of durable knowledge (summaries, lessons, playbooks) | Workspace memory blobs |
| **Project** | Unit of context (scopes memory, threads, missions) | Flat workspace namespace |

## Crate Structure

Single crate: `crates/ironclaw_engine/`

```
crates/ironclaw_engine/
  Cargo.toml
  CLAUDE.md
  src/
    lib.rs                    # Public API, re-exports

    types/                    # Core data structures (no async, no I/O)
      mod.rs
      error.rs                # EngineError, ThreadError, StepError, CapabilityError
      thread.rs               # Thread, ThreadId, ThreadState, ThreadType, ThreadConfig
      step.rs                 # Step, StepId, StepStatus, ExecutionTier, ActionCall, ActionResult, LlmResponse
      capability.rs           # Capability, ActionDef, EffectType, CapabilityLease, PolicyRule
      memory.rs               # MemoryDoc, DocId, DocType
      project.rs              # Project, ProjectId
      event.rs                # ThreadEvent, EventKind (16 variants for event sourcing)
      provenance.rs           # Provenance enum (User, System, ToolOutput, LlmGenerated, etc.)
      message.rs              # ThreadMessage, MessageRole
      conversation.rs         # ConversationSurface, ConversationEntry (Phase 5)
      mission.rs              # Mission, MissionId (Phase 4)

    traits/                   # External dependency abstractions (host implements these)
      mod.rs
      llm.rs                  # LlmBackend trait
      store.rs                # Store trait (18 CRUD methods)
      effect.rs               # EffectExecutor trait

    capability/               # Capability management
      mod.rs
      registry.rs             # CapabilityRegistry
      lease.rs                # LeaseManager (grant, check, consume, revoke, expire)
      policy.rs               # PolicyEngine (deterministic effect-level allow/deny/approve)
      provenance.rs           # ProvenanceTracker (taint analysis, Phase 4)

    runtime/                  # Thread lifecycle management
      mod.rs
      manager.rs              # ThreadManager (spawn, supervise, stop, inject, join)
      tree.rs                 # ThreadTree (parent-child relationships)
      messaging.rs            # ThreadSignal, ThreadOutcome, signal channels
      conversation.rs         # ConversationManager (Phase 5)

    executor/                 # Step execution
      mod.rs
      loop_engine.rs          # ExecutionLoop (core loop, handles Text/ActionCalls/Code)
      structured.rs           # Tier 0: structured tool calls
      scripting.rs            # Tier 1: embedded Python via Monty (RLM pattern)
      context.rs              # Context builder (messages + actions from leases)
      intent.rs               # Tool intent nudge detection

    memory/                   # Memory document system
      mod.rs
      store.rs                # MemoryStore (project-scoped doc CRUD)
      retrieval.rs            # RetrievalEngine (stub, Phase 4)

    reflection/               # Post-thread reflection (stub, Phase 4)
      mod.rs
```

Dependencies:
- `tokio` (sync, time, macros, rt), `serde` + `serde_json`, `thiserror`, `tracing`, `uuid`, `chrono`, `async-trait`
- `monty` (git dep from pydantic/monty) — embedded Python interpreter for CodeAct

---

## Phase 1: Foundation — DONE

**Commit:** `8be19a4`

All core types, trait definitions, and thread state machine. 32 tests.

- Types: Thread (state machine), Step (LlmResponse, ActionCall, ActionResult, TokenUsage), Capability (ActionDef, EffectType, CapabilityLease, PolicyRule), MemoryDoc (DocType), Project, ThreadEvent (EventKind), ThreadMessage, Provenance, EngineError
- Traits: LlmBackend, Store (18 methods), EffectExecutor
- Tests: state machine transitions (valid/invalid), lease expiry (time/use), message constructors

---

## Phase 2: Execution Engine (Tier 0) — DONE

**Commit:** `bf7dfb8`

Working execution loop equivalent to `run_agentic_loop()`. 74 tests.

- **CapabilityRegistry** — register/get/list capabilities and actions (5 tests)
- **LeaseManager** — grant, check, consume, revoke, expire. `RwLock<HashMap>` (7 tests)
- **PolicyEngine** — deterministic: global policies → capability policies → action requires_approval → effect type. Deny > RequireApproval > Allow (8 tests)
- **ThreadTree** — parent-child relationships (5 tests)
- **ThreadSignal/ThreadOutcome** — mpsc-based inter-thread messaging
- **ThreadManager** — spawn as tokio tasks, stop, inject messages, join (3 tests)
- **ExecutionLoop** — signals → context → LLM call → handle Text/ActionCalls → record step + events → repeat (6 tests)
- **execute_action_calls()** — lease lookup → policy → consume → EffectExecutor
- **signals_tool_intent()** — nudge detection (6 tests)
- **MemoryStore** + **RetrievalEngine** — stubs for Phase 4

---

## Phase 3: CodeAct Executor (Tier 1 — Monty + RLM) — DONE

**Commits:** `b59a0b9`, `9538332`

LLMs write Python code that composes tools, queries thread context as data, and recursively spawns sub-agents. Uses Monty interpreter with the RLM (Recursive Language Model) pattern.

### What was built

**Monty integration** (`executor/scripting.rs`):
- Embeds Pydantic's Monty Python interpreter (git dep, v0.0.8)
- `MontyRun::new(code, "step.py", input_names)` → `runner.start(inputs, tracker, print)` → loop over `RunProgress` suspension points
- Resource limits: 30s timeout, 64MB memory, 1M allocations, recursion depth 1000
- All execution wrapped in `catch_unwind` (Monty can panic)
- `monty_to_json()` / `json_to_monty()` bidirectional conversion

**RLM features** (cross-referenced against official RLM, fast-rlm, Prime Intellect):

| Feature | Implementation | Reference |
|---|---|---|
| Context as variables | `context`, `goal`, `step_number`, `previous_results` injected as Monty inputs | RLM paper §3 |
| `FINAL(answer)` | FunctionCall handler sets `final_answer`, loop exits | Official RLM, fast-rlm |
| `FINAL_VAR(name)` | FunctionCall handler stores var name reference | Official RLM |
| `llm_query(prompt, context)` | FunctionCall → single-shot `LlmBackend::complete()` with force_text | All three impls |
| `llm_query_batched(prompts)` | FunctionCall → parallel `tokio::spawn` for each prompt, collect results | fast-rlm asyncio.gather, Prime Intellect llm_batch |
| Output truncation (8K chars) | `compact_output_metadata()` with `[TRUNCATED: last N chars]` or `[FULL OUTPUT]` prefix | Prime Intellect 8192, Official 20K, fast-rlm 2K |
| Step 0 orientation | Auto-inject context metadata (msg count, total chars, goal, preview) before first code step | fast-rlm Step 0 auto-print |
| Error-to-LLM flow | Parse/runtime/name/OS errors return as stdout content, not EngineError. LLM can self-correct. | Official RLM (errors in stderr shown to LLM) |
| Tool dispatch | Unknown functions suspend VM → lease → policy → EffectExecutor → resume | Original design |
| OS call denial | `RunProgress::OsCall` → `OSError` exception | Original design |
| Async denial | `RunProgress::ResolveFutures` → error in stdout | Original design |

**LlmResponse::Code** variant + **ExecutionTier::Scripting** — the `ExecutionLoop` routes `Code` to `scripting::execute_code()`.

### Remaining gaps (future phases)

| Gap | Where it fits | Source |
|---|---|---|
| `rlm_query()` (child gets own REPL + full RLM loop) | Phase 4 — needs ThreadManager in CodeAct runtime | Official RLM |
| Dual model routing (cheaper model for sub-calls) | Phase 4 — LlmBackend needs `complete_with_model(model, ...)` | fast-rlm, Official RLM |
| Compaction at 85% context limit | Phase 4 — summarize history, reset messages | Official RLM |
| Persistent REPL state across code steps | Monty limitation (fresh MontyRun per step) — monitor Monty roadmap | Official RLM LocalREPL |
| Scaffold restoration (prevent code overwriting context/llm_query) | Not needed — Monty creates fresh execution per step | Official RLM |
| `SHOW_VARS()` listing | Monty limitation — no namespace access from host | Official RLM |
| Consecutive error counting + threshold | Phase 4 — add `max_consecutive_errors` to ThreadConfig | Official RLM |
| USD budget tracking | Phase 4 — needs cost data from LlmBackend | Official RLM, fast-rlm |
| answer dictionary pattern (`{"content":"","ready":True}`) | Alternative to FINAL() — lower priority, FINAL() works | Prime Intellect |
| Tools restricted to sub-LLMs only | Design decision for Phase 4 — evaluate tradeoffs | Prime Intellect |
| Lazy Path objects (data on disk until accessed) | Phase 4 retrieval — avoid loading full context upfront | Google ADK |
| Pass-by-reference chunk IDs for sub-agents | Phase 4 retrieval — sub-agents get IDs not content | rlm-rs |
| Code-aware chunking (tree-sitter) | Phase 4 retrieval — for code repositories | rlm-rs |

---

## Phase 4: Memory, Reflection, and Learning

**Goal:** The agent learns from its work. Completed threads produce structured knowledge. Context building uses project-scoped retrieval, not raw history replay.

### 4.1 Project-scoped retrieval
- `RetrievalEngine::retrieve_context(project_id, query, max_docs)` — keyword + semantic search over project's memory docs
- Context builder: thread state + project docs (summaries, lessons, playbooks) + capability descriptions
- **Lazy loading** (Google ADK pattern): data stays in storage until code explicitly accesses it via variables
- **Pass-by-reference** (rlm-rs pattern): sub-agents receive chunk IDs, fetch content on demand

### 4.2 Reflection pipeline
After thread completes (state → Completed), optionally spawns a Reflection-type thread:
1. **Summarize** → `DocType::Summary`
2. **Extract lessons** → `DocType::Lesson` (from failures, workarounds, discoveries)
3. **Detect issues** → `DocType::Issue` (unresolved problems)
4. **Detect missing capabilities** → `DocType::Spec` ("no tool available" patterns)
5. **Promote playbooks** → `DocType::Playbook` (successful multi-step procedures)

Reflection is itself a thread running CodeAct — it's recursive.

### 4.3 Compaction (from RLM)
When message history tokens reach **85% of model context limit** (per official RLM):
1. Ask LLM to "summarize progress so far" with instructions to preserve intermediate results
2. Replace message history with `[system, summary, "continue..."]`
3. Append full trajectory to a `history` variable accessible from code
- Requires token counting — add `count_tokens(messages, model)` utility (tiktoken or char-estimate fallback, per official RLM `token_utils.py`)

### 4.4 `rlm_query()` — full recursive sub-agent
Unlike `llm_query()` (single-shot text completion), `rlm_query(prompt)` spawns a **child thread with its own CodeAct executor**:
- Child gets own REPL, own context variable, own iteration budget
- Child can call `llm_query()` and tools but NOT `rlm_query()` (depth limit)
- Budget/timeout inheritance: child gets `remaining_budget - spent`, `remaining_timeout - elapsed`
- Returns child's `FINAL()` answer as a string variable

### 4.5 Dual model routing
`LlmBackend` gains optional depth-based model selection:
- depth=0 (root): use primary model (e.g., GPT-5, Claude Opus)
- depth=1+ (sub-calls): use cheaper model (e.g., GPT-5-mini, Claude Haiku)
- Configurable via `ThreadConfig` or `LlmCallConfig`

### 4.6 Budget controls (from RLM cross-reference)
Add to `ThreadConfig`:
- `max_budget_usd: Option<f64>` — cumulative USD cost limit (needs cost data from LlmBackend)
- `max_timeout: Option<Duration>` — wall-clock timeout for entire thread
- `max_tokens_total: Option<u64>` — cumulative input+output token limit
- `max_consecutive_errors: Option<u32>` — consecutive steps with errors before termination
- All limits inherited by child threads with remaining budget

### 4.7 Provenance tracking
Every data value tagged with origin. Policy engine uses provenance at effect boundaries:
- LlmGenerated → Financial effects: require approval
- ToolOutput from untrusted sources: extra validation
- User-provenance: trusted

### 4.8 Missions (long-running goals)
```rust
pub struct Mission {
    pub id: MissionId,
    pub project_id: ProjectId,
    pub goal: String,
    pub status: MissionStatus, // Active, Paused, Completed, Failed
    pub cadence: MissionCadence, // Cron, OnEvent, OnPush, Manual
    pub thread_history: Vec<ThreadId>,
    pub success_criteria: Option<String>,
}
```

### 4.9 Tool reliability learning
Track per-action EMA metrics (success rate, latency, failure patterns). Feed into context builder.

### 4.10 Tests
- Reflection produces correct doc types from a completed thread with failures
- Retrieval returns project-scoped docs, not cross-project
- Compaction triggers at 85% context, preserves intermediate results
- `rlm_query()` spawns child thread, returns answer, respects budget inheritance
- Dual model routing: root uses primary, sub-calls use cheaper
- Budget exceeded → `BudgetExceededError` with partial answer
- Consecutive errors threshold → termination
- Provenance taint blocks financial effects from LLM-generated data
- Mission spawns thread on cadence, tracks history

---

## Phase 5: Conversation Surface + Multi-Channel Integration

**Goal:** Conversations (UI) are cleanly separated from threads (execution). Multiple channels route to the same thread model.

### 5.1 ConversationSurface
```rust
pub struct ConversationSurface {
    pub id: ConversationId,
    pub channel: String,        // "telegram", "slack", "web", "cli"
    pub user_id: String,
    pub entries: Vec<ConversationEntry>,
    pub active_threads: Vec<ThreadId>,
}

pub struct ConversationEntry {
    pub id: EntryId,
    pub sender: EntrySender,    // User or Agent
    pub content: String,
    pub origin_thread_id: Option<ThreadId>,
    pub timestamp: DateTime<Utc>,
}
```

### 5.2 ConversationManager
- Routes incoming channel messages to conversation surfaces
- User message → may spawn new foreground thread or inject into existing
- Multiple threads can be active simultaneously per conversation
- Thread outputs (replies, status updates) appear as conversation entries

### 5.3 Channel adaptation
The existing `Channel` trait stays. A bridge adapter translates:
- `IncomingMessage` → `ConversationEntry` → spawn/inject `Thread`
- `ThreadOutcome` → `ConversationEntry` → `OutgoingResponse`
- `StatusUpdate` events → `ConversationEntry` with metadata

### 5.4 Tests
- Two concurrent threads in one conversation → entries interleaved correctly
- Thread outlives conversation (background) → results appear when user returns
- Channel-agnostic: same thread model works for Telegram, Web, CLI

---

## Phase 6: Main Crate Integration

**Goal:** Bridge adapters connect the engine to existing IronClaw infrastructure. Prove the engine works end-to-end via acceptance tests.

### 6.1 Bridge adapters (`src/bridge/`)
- `LlmBridgeAdapter` — wraps `Arc<dyn LlmProvider>`, converts `ThreadMessage` ↔ `ChatMessage`, `ActionDef` ↔ `ToolDefinition`. Implements depth-based model routing via existing `cheap_llm` in `AgentDeps`
- `StoreBridgeAdapter` — wraps `Arc<dyn Database>`, maps engine CRUD to existing sub-traits. New tables for threads/projects/docs/leases/events (migration V14+)
- `EffectBridgeAdapter` — wraps `ToolRegistry` + `SafetyLayer`. On `execute_action()`: lookup tool → validate params → execute → sanitize output → return. Safety logic lives here, not in the engine

### 6.2 Database migrations
New tables (both PostgreSQL and libSQL):
- `engine_threads`, `engine_steps`, `engine_events`, `engine_projects`, `engine_memory_docs`, `engine_capability_leases`

### 6.3 Integration strategy
Implement `EngineV2Delegate` that wraps `ExecutionLoop` but presents the `LoopDelegate` interface. The existing dispatcher calls `run_agentic_loop()` with either ChatDelegate or EngineV2Delegate. This enables gradual migration without a flag day.

### 6.4 Two-phase commit for high-stakes effects
For `WriteExternal` + `Financial` effects:
1. **Simulate** — dry-run, return preview
2. **Approve** — user or policy approves
3. **Execute** — actual effect

Commit policies: `Direct` (ReadLocal, ReadExternal), `Approved` (WriteExternal), `TwoPhase` (Financial, production deploys). Implemented in `EffectBridgeAdapter` at the boundary.

### 6.5 Acceptance testing
Use existing `TestRig` + `TraceLlm`:
- Load pre-recorded LLM trace fixtures
- Drive engine via bridge adapters
- Compare output with `verify_trace_expects()`
- All existing fixture tests must pass

When all tests pass: make engine the default, deprecate old path.

### 6.6 Tests
- Bridge adapter conversion: ThreadMessage ↔ ChatMessage round-trips
- End-to-end: TestRig drives engine, same output as old loop
- Migration: new tables for both backends
- Two-phase commit: simulate returns preview, approve triggers execution

---

## Phase 7: Cleanup and Migration

**Goal:** Remove old abstractions, migrate all code to engine model.

### 7.1 Deprecate old types
- `Session` / `Thread` / `Turn` → engine `Thread` + `Step`
- `JobState` / `JobContext` → engine `ThreadState` + `Thread`
- `RoutineEngine` / `Routine` → engine `Mission` + `Thread`
- `SkillSelector` / `LoadedSkill` → engine `Capability` (knowledge)
- `HookPipeline` → engine `Capability` (policies)
- `ApprovalRequirement` / `ApprovalContext` → engine `CapabilityLease` + `PolicyEngine`

### 7.2 Slim down main crate
- Agent module becomes thin adapter over engine
- `app.rs` orchestrates engine startup
- Remove `LoopDelegate` and its three implementations
- Remove `SessionManager`, `Scheduler` (replaced by `ThreadManager`)

### 7.3 Sub-crate extraction
Once boundaries stabilize, split if beneficial:
- `ironclaw_types` — shared types for WASM extensions
- `ironclaw_capability` — if used by tooling/CLI independently

---

## Phase 8: Sandboxed Execution + Infrastructure Integration

**Goal:** Leverage existing IronClaw infrastructure for sandboxed execution. This is NOT about running CodeAct/RLM in different runtimes — Monty is the sole Python executor. This is about isolating threads and running third-party tools safely.

### 8.1 WASM tool sandbox (existing infrastructure)
- Third-party tools from `tools-src/` and the registry run in WASM via existing `src/tools/wasm/`
- The engine's `EffectExecutor` bridge routes tool calls to WASM-sandboxed tools transparently
- No change to the engine crate — this is purely adapter-layer routing in `EffectBridgeAdapter`
- Fuel metering, memory limits, network allowlisting all come from existing `wasmtime` infrastructure

### 8.2 Docker thread isolation
- Threads tagged with `ThreadType::Research` or high-compute tasks can optionally execute inside Docker containers via existing `src/sandbox/` infrastructure
- The `ThreadManager` bridge decides whether to spawn a thread in-process or in a container based on the thread's capability leases (if it needs `Compute` or `WriteExternal` effects, sandbox it)
- Inside the container: Monty still executes the Python code, but the entire thread runs in isolation with credential injection via the sandbox proxy
- Maps to existing `ContainerDelegate` pattern but unified under the thread model

### 8.3 WASM channel sandbox (existing infrastructure)
- Channel implementations (Telegram, Slack, Discord, etc.) continue running as WASM modules via existing `src/channels/wasm/`
- `ConversationManager` bridge routes channel messages through existing `ChannelManager` → WASM channel → engine thread

### 8.4 Tests
- WASM tool executes through EffectBridgeAdapter with fuel limits
- Docker-isolated thread completes and returns outcome to parent
- Channel WASM module produces entries in ConversationSurface

---

## Cross-Cutting Concerns

### Security Model
- **Capability leases** replace static permissions. Scoped, time-limited, use-limited. Blast radius bounded
- **Effect typing** on every action. Policy engine uses effect types for allow/deny
- **Provenance tracking** (Phase 4). Taint analysis at effect boundaries
- **Two-phase commit** (Phase 6) for WriteExternal + Financial effects at the adapter boundary
- **Safety at adapter boundary**. Engine is pure orchestration; `SafetyLayer` applied in `EffectBridgeAdapter`
- **Monty sandboxing**: no filesystem (OsCall denied), no network (no imports), resource-limited, catch_unwind for panics. Monty is the sole CodeAct/RLM executor — no need for WASM/Docker Python runtimes
- **WASM for third-party tools** (Phase 8). Untrusted tool code runs in wasmtime sandbox with fuel metering
- **Docker for thread isolation** (Phase 8). High-risk threads run in containers with credential injection

### Observability
- **Event sourcing** replaces ad-hoc `ObserverEvent`. Every thread has complete event log (16 event kinds)
- **Trace-based testing** (Phase 4+). Event logs as golden traces
- **Thread-structural events** (thread.started, step.completed, action.executed) vs per-subsystem

### RLM Execution Model
- **Context as variable**: thread messages/goal/results injected as Python variables, not LLM attention input
- **Output truncation**: 8K chars between steps (configurable), with `[TRUNCATED]`/`[FULL OUTPUT]` prefixes
- **Step 0 orientation**: auto-inject context metadata before first code step
- **FINAL()/FINAL_VAR()**: explicit termination from within code
- **llm_query()/llm_query_batched()**: recursive/parallel sub-agent calls
- **Error transparency**: Python errors flow to LLM for self-correction, not step termination
- **Symbolic composition**: sub-agent results stored as variables, not injected into parent context

### Backward Compatibility
- Engine runs alongside existing code via `EngineV2Delegate` adapter
- Bridge adapters translate between engine and existing types
- WASM tools/channels unchanged (bridge wraps `Tool`/`Channel` traits)
- MCP tools unchanged (same adapter principle)
- Existing tests unmodified — they test the old path

---

## Implementation Progress

| Phase | Scope | Status | Tests | Commits |
|-------|-------|--------|-------|---------|
| **1** | Types + traits + state machine | **DONE** | 32 | `8be19a4` |
| **2** | Tier 0 executor + capability + runtime | **DONE** | 74 | `bf7dfb8` |
| **3** | CodeAct (Monty + RLM pattern) | **DONE** | 74 | `b59a0b9`, `9538332` |
| **4** | Budget controls + compaction + reflection | **DONE** | 78 | `4bc7ffd` |
| **5** | Conversation surface | **DONE** | 85 | `0827235` |
| **6** | Main crate bridge + acceptance tests | Next | — | — |
| **7** | Cleanup + migration | Planned | — | — |
| **8** | WASM tools + Docker isolation | Planned | — | — |

Phase 6 depends on Phases 1-5 being stable. Phase 7 depends on Phase 6 passing acceptance. Phase 8 is infrastructure integration with existing WASM/Docker systems.

---

## Verification (per phase)

```bash
# Engine crate only:
cargo check -p ironclaw_engine
cargo clippy -p ironclaw_engine --all-targets -- -D warnings
cargo test -p ironclaw_engine

# Full workspace (no regressions):
cargo check
cargo clippy --all --benches --tests --examples --all-features
cargo test

# Phase 7+ acceptance:
cargo test  # engine-driven tests match existing fixtures via EngineV2Delegate
```
