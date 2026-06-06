# Agent Map — ironclaw_engine

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file (five primitives, thread state machine, execution loop, capability leases, data-retention rule).
- Read `MONTY.md` before touching Tier-1 CodeAct/scripting; it documents the embedded Python interpreter's pin and supported-feature limits.
- Read `Cargo.toml` for dependencies (internally only `ironclaw_common` and `ironclaw_skills`) and note the ReDoS guardrail at the top of `src/lib.rs`: do not add `fancy-regex` without redesigning `executor/orchestrator.rs::__regex_match__`.
- Full roadmap: `docs/plans/2026-03-20-engine-v2-architecture.md`; per-project sandbox: `docs/plans/2026-04-10-engine-v2-sandbox.md`.

## What This Crate Owns

- The unified thread-capability-CodeAct execution engine (engine v2), which replaces ~10 legacy abstractions with five primitives — Thread, Step, Capability, MemoryDoc, Project. Currently:
- Core data types (`types`, no async/no I/O): `Thread`/`Step`/`Capability`/`MemoryDoc`/`Project` and their IDs, `ThreadEvent`/`EventKind`, `ThreadMessage`/`MessageRole`, `Mission`/`MissionCadence`/`MissionStatus`, `Provenance`, conversation surfaces, and the `EngineError`/`ThreadError`/`StepError`/`CapabilityError` family.
- External-dependency traits the host implements via bridge adapters (`traits`): `LlmBackend` (over `LlmProvider`), `Store` (over the `Database` backends), `EffectExecutor` (over `ToolRegistry` + `SafetyLayer`), and `WorkspaceReader`.
- Capability management (`capability`): `CapabilityRegistry`, `LeaseManager`, `LeasePlanner`/`CapabilityGrantPlan`, and the deterministic `PolicyEngine`/`PolicyDecision` (`Deny > RequireApproval > Allow`, with provenance taint).
- Gate pipeline (`gate`): `GatePipeline`, `LeaseGate`, `ExecutionGate`/`GateController`/`GateDecision`/`GateResolution`/`ResumeKind`, and tool-tier classification (`ToolTier`, `classify_tool_tier`).
- Step execution (`executor`): `ExecutionLoop` (replaces the legacy agentic loop), Tier-0 structured tool calls (`structured`) and Tier-1 CodeAct via Monty (`scripting`, `orchestrator`), context/prompt building (`context`, `thread_context`, `prompt`), and execution trace recording (`trace`).
- Thread lifecycle runtime (`runtime`): `ThreadManager`, `ConversationManager`, `MissionManager` (learning missions), `ThreadTree`, signal/`ThreadOutcome` messaging, lease refresh, and internal writes.
- Memory document system (`memory`): `MemoryStore`, `RetrievalEngine`, `SkillTracker`.
- Workspace mounts (`workspace`): `MountBackend`, `ProjectMounts`/`WorkspaceMounts`, `ProjectMountFactory`.
- `ReliabilityTracker` (per-action EMA success/latency) and the prompt templates in `prompts/*.md` (loaded via `include_str!`).

## Do Not Move In Here

- A dependency on the main `ironclaw` crate, product transport/channels, or UI behavior — the engine is testable in isolation.
- Safety logic (sanitization, leak detection): applied at the `EffectExecutor` adapter boundary in the host, not here.
- Provider-specific LLM auth or concrete `Store`/database backends: those are host bridge adapters behind the traits.
- Deletion of LLM output. Thread messages, steps, and events are never deleted; in-memory `Store` HashMaps are a cache that evicts to bound RAM, but `load_thread`/`load_steps`/`load_events` must fall back to the database.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_engine`
- Lint: `cargo clippy -p ironclaw_engine --all-targets -- -D warnings`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- After changing a trait surface (`LlmBackend`/`Store`/`EffectExecutor`), add caller-level tests in the host bridge adapters that implement them.

## Agent Notes

- Thread state transitions go through `ThreadState::can_transition_to()`; terminal states are `Done` and `Failed`.
- Tier-1 CodeAct follows the RLM pattern: context-as-variables (not attention input), recursive `llm_query()`, and compact output metadata between steps. Execution is bounded (30s / 64MB / 1M allocations) and wrapped in `catch_unwind` for Monty panic safety.
- Installed-but-unauthed provider tools are direct-callable: the auth preflight raises an `Authentication` gate at execute time and the OAuth callback resumes the parked VM. Tools needing user-driven setup (`NeedsSetup`, `Inactive`, `AvailableNotInstalled`) are surfaced under `Activatable Integrations`; the model cannot enable them itself.
- `MissionManager::ensure_learning_missions()` idempotently *bootstraps* five learning missions at project setup — `self-improvement`, `skill-repair`, `skill-extraction`, `conversation-insights`, and `expected-behavior` — it registers them, it does not fire them. Firing is event-driven through `fire_on_system_event()` (wired by `start_event_listener()`): the first four key off engine events (e.g. `thread_completed_with_skill_gap` / `thread_completed_with_learnings` / `conversation_insights_due`), while `expected-behavior` keys off a `user_feedback` / `expected_behavior` event, not thread completion.
- Keep multi-line prompt templates in `prompts/*.md`, never inline as Rust string constants.
