# IronClaw Engine Crate

Unified thread-capability-CodeAct execution model. Replaces ~10 separate abstractions (Session, Job, Routine, Channel, Tool, Skill, Hook, Observer, Extension, LoopDelegate) with 5 primitives.

## Full Architecture Plan

See `docs/plans/2026-03-20-engine-v2-architecture.md` for the 8-phase roadmap.

## Five Primitives

| Primitive | Purpose | Replaces |
|-----------|---------|----------|
| **Thread** | Unit of work with lifecycle, parent-child tree, capability leases | Session + Job + Routine + Sub-agent |
| **Step** | Unit of execution (one LLM call + its action executions) | Agentic loop iteration + tool calls |
| **Capability** | Unit of effect (actions + knowledge + policies) | Tool + Skill + Hook + Extension |
| **MemoryDoc** | Unit of durable knowledge (summaries, lessons, skills) | Workspace memory blobs |
| **Project** | Unit of context (scopes memory, threads, missions) | Flat workspace namespace |

## Build & Test

```bash
cargo check -p ironclaw_engine
cargo clippy -p ironclaw_engine --all-targets -- -D warnings
cargo test -p ironclaw_engine
```

## Module Map

```
src/
├── lib.rs                # Public API, re-exports
├── types/                # Core data structures (no async, no I/O)
│   ├── thread.rs         # Thread, ThreadId, ThreadState (state machine), ThreadType, ThreadConfig
│   ├── step.rs           # Step, StepId, LlmResponse, ActionCall, ActionResult, TokenUsage
│   ├── capability.rs     # Capability, ActionDef, EffectType, CapabilityLease, PolicyRule
│   ├── memory.rs         # MemoryDoc, DocId, DocType (Summary/Lesson/Skill/Issue/Spec/Note)
│   ├── project.rs        # Project, ProjectId
│   ├── event.rs          # ThreadEvent, EventKind (18 variants for event sourcing)
│   ├── message.rs        # ThreadMessage, MessageRole
│   ├── provenance.rs     # Provenance enum (User/System/ToolOutput/LlmGenerated/etc.)
│   ├── conversation.rs   # ConversationSurface, ConversationEntry, EntrySender
│   ├── mission.rs        # Mission, MissionId, MissionCadence, MissionStatus
│   └── error.rs          # EngineError, ThreadError, StepError, CapabilityError
├── traits/               # External dependency abstractions (host implements these)
│   ├── llm.rs            # LlmBackend trait
│   ├── store.rs          # Store trait (20 CRUD methods)
│   └── effect.rs         # EffectExecutor trait
├── capability/           # Capability management
│   ├── registry.rs       # CapabilityRegistry — register/get/list capabilities
│   ├── lease.rs          # LeaseManager — grant/check/consume/revoke/expire leases
│   ├── policy.rs         # PolicyEngine — deterministic effect-level allow/deny/approve + provenance taint
│   └── skill_tracker.rs  # SkillTracker — confidence tracking, versioned updates, rollback
├── runtime/              # Thread lifecycle management
│   ├── manager.rs        # ThreadManager — spawn, stop, inject messages, join threads
│   ├── conversation.rs   # ConversationManager — routes UI messages to threads
│   ├── mission.rs        # MissionManager — long-running goals that spawn threads on cadence
│   ├── tree.rs           # ThreadTree — parent-child relationships
│   └── messaging.rs      # ThreadSignal, ThreadOutcome, signal channels
├── executor/             # Step execution
│   ├── loop_engine.rs    # ExecutionLoop — core loop replacing run_agentic_loop()
│   ├── structured.rs     # Tier 0: structured tool call execution
│   ├── scripting.rs      # Tier 1: embedded Python via Monty (CodeAct/RLM)
│   ├── context.rs        # Context builder (messages + actions from leases + memory docs)
│   ├── compaction.rs     # Context compaction when approaching model context limit
│   ├── prompt.rs         # System prompt construction (CodeAct preamble/postamble)
│   ├── intent.rs         # Tool intent nudge detection
│   └── trace.rs          # Execution trace recording and retrospective analysis
├── memory/               # Memory document system
│   ├── store.rs          # MemoryStore — project-scoped doc CRUD
│   └── retrieval.rs      # RetrievalEngine — keyword-based context retrieval from project docs
└── reliability.rs        # ReliabilityTracker — per-action success rate and latency via EMA
```

## Thread State Machine

```
Created → Running → Waiting → Running (resume)
                  → Suspended → Running (resume)
                  → Completed → Done
                  → Failed
```

Validated by `ThreadState::can_transition_to()`. Terminal states: `Done`, `Failed`.

## Learning Missions

Three event-driven missions fire automatically after thread completion:

1. **Error diagnosis** (`self-improvement`) — fires when a thread completes with trace issues. Diagnoses root cause and applies prompt overlays or orchestrator patches.
2. **Skill extraction** (`skill-extraction`) — fires when a thread succeeds with 5+ steps and 3+ tool actions. Extracts reusable skills with activation metadata, CodeAct code snippets, and domain tags. Output stored as `DocType::Skill` MemoryDoc.
3. **Conversation insights** (`conversation-insights`) — fires every 5 completed threads in a project. Extracts user preferences, domain knowledge, and workflow patterns.

Created by `MissionManager::ensure_learning_missions()` at project bootstrap.

## External Trait Boundaries

The engine defines three traits that the host crate implements:

| Trait | Purpose | Host wraps |
|-------|---------|------------|
| `LlmBackend` | `complete(messages, actions, config) -> LlmOutput` | `LlmProvider` |
| `Store` | Thread/Step/Event/Project/Doc/Lease CRUD | `Database` (PostgreSQL + libSQL) |
| `EffectExecutor` | `execute_action(name, params, lease, ctx) -> ActionResult` | `ToolRegistry` + `SafetyLayer` |

## Execution Loop

`ExecutionLoop::run()` handles three `LlmResponse` variants:

1. Check signals (Stop, InjectMessage) via `mpsc::Receiver`
2. Build context (messages + available actions from active leases)
3. Call LLM via `LlmBackend::complete()`
4. **If `Text`**: check tool intent nudge, return if final response
5. **If `ActionCalls`** (Tier 0): for each call, find lease → check policy → consume use → execute via `EffectExecutor` → record result
6. **If `Code`** (Tier 1): execute Python via Monty with context-as-variables and `llm_query()` support → compact metadata in context
7. Record Step, emit ThreadEvents
8. Repeat until: text response, stop signal, max iterations, or approval needed

## CodeAct / Monty Integration (Tier 1)

Python execution via Monty interpreter (`executor/scripting.rs`). Follows the RLM (Recursive Language Model) pattern.

**Context as variables** (not attention input):
- Thread messages injected as `context` Python variable
- Thread goal as `goal`, step index as `step_number`
- Prior action results as `previous_results` dict
- The LLM's chat context stays lean; full data lives in REPL variables

**Tool dispatch**: Unknown function calls suspend the VM → lease check → policy check → `EffectExecutor` → result returned to Python.

**`llm_query(prompt, context)`**: Recursive subagent call. Suspends VM → spawns single-shot LLM call → returns text result as Python string. Results stay as variables (symbolic composition), not injected into parent's attention window.

**Compact output metadata**: Between code steps, only a summary is added to chat context (`"[code output] stdout (4532 chars): The results show..."`) — not the full output. This prevents context bloat across iterations.

**Resource limits**: 30s timeout, 64MB memory, 1M allocations. All execution wrapped in `catch_unwind` for Monty panic safety.

## Capability Leases

Threads don't have static permissions. They receive **leases** — scoped, time-limited, use-limited grants:

```rust
CapabilityLease {
    thread_id, capability_name, granted_actions,
    expires_at: Option<DateTime>,  // time-limited
    max_uses: Option<u32>,         // use-limited
    revoked: bool,
}
```

The `PolicyEngine` evaluates actions against leases deterministically: `Deny > RequireApproval > Allow`.

## Effect Types

Every action declares its side effects. The policy engine uses these for allow/deny:

```
ReadLocal, ReadExternal, WriteLocal, WriteExternal,
CredentialedNetwork, Compute, Financial
```

## Key Design Decisions

1. **No dependency on main `ironclaw` crate** — clean separation, testable in isolation
2. **No safety logic** — sanitization/leak detection is applied at the adapter boundary (`EffectExecutor` impl)
3. **Event sourcing from day one** — every thread records a complete event log via `ThreadEvent`
4. **Tier 0 + Tier 1** — structured tool calls (Tier 0) and embedded Python via Monty (Tier 1, CodeAct)
5. **Engine owns its message type** — `ThreadMessage` is simpler than `ChatMessage`; bridge adapters handle conversion
6. **RLM pattern** — context as variable (not attention input), recursive `llm_query()`, compact output metadata between steps

## Code Style

Follows the main crate's conventions from `/CLAUDE.md`:
- No `.unwrap()` or `.expect()` in production code (tests are fine)
- `thiserror` for error types
- Map errors with context
- Prefer strong types over strings (newtypes for IDs)
- All I/O is async with tokio
- `Arc<T>` for shared state, `RwLock` for concurrent access
