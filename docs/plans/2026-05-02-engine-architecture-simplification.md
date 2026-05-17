# IronClaw Engine: Argument & Layer Simplification

**Date:** 2026-05-02
**Status:** Proposed
**Scope:** `crates/ironclaw_engine/`
**Goal:** Reduce dependency-arg threading and collapse duplicate dispatch
paths in the engine without changing externally visible behavior.

---

## Diagnosis

The five primitives (Thread, Step, Capability, MemoryDoc, Project) are
clean. The runtime plumbing around them is not. Five symptoms point at the
same root cause: there is no aggregation type for engine-wide services,
so every layer re-threads the same Arcs.

### Symptom 1 — Service Arcs replicated at every layer

Six dependencies — `LlmBackend`, `EffectExecutor`, `Store`,
`CapabilityRegistry`, `LeaseManager`, `PolicyEngine` — appear at:

- `runtime/manager.rs:34-47` (stored as fields on `ThreadManager`)
- `executor/loop_engine.rs:105-124` (re-passed into `ExecutionLoop`)
- `executor/orchestrator.rs:436-449` (re-passed as 12 args to
  `execute_orchestrator`, gated by `#[allow(clippy::too_many_arguments)]`)
- `runtime/mission.rs` (subset, plus optional `EffectExecutor`)

Three layers, identical payload, no shared name.

### Symptom 2 — `with_*` builders hide required dependencies

`ExecutionLoop::new` takes 5 Arcs; five further deps come in via
`with_capabilities`, `with_store`, `with_retrieval`, `with_event_tx`,
`with_platform_info` (`loop_engine.rs:152-186`). Production wires all
five. Tests wire some. The type system says "optional" so the runtime
carries `is_some()` branches forever, even though the production
invariant is "always present." Dead paths are bug homes.

### Symptom 3 — `Thread` and `ThreadExecutionContext` duplicate state

`ThreadExecutionContext` (`traits/effect.rs:21-45`) carries `thread_id`,
`thread_type`, `project_id`, `user_id` — all already on `Thread`. It
also carries `available_actions_snapshot` and
`available_action_inventory_snapshot` that the orchestrator stuffs back
mid-step to redeliver work the loop already did. Two sources of truth
for "what is executing right now."

### Symptom 4 — Two parallel action-dispatch pipelines

- Tier 0 (`executor/structured.rs:55-67`): per call → fetch lease →
  policy check → `effects.execute_action`.
- Tier 1 (`executor/orchestrator.rs` host-fn match around line 534+,
  plus `executor/scripting.rs`): same shape, written separately.

Every safety/policy/lease change has to land in both. Recurring bug
shape: PRs #2470, #2491, #2676.

### Symptom 5 — Manager-of-managers stack

`ConversationManager → ThreadManager → ExecutionLoop →
execute_orchestrator → host-fn handlers`. Five frames before a tool
runs. `runtime/mission.rs` is 7,933 lines; `executor/orchestrator.rs`
is 5,212. Length is the layering tax made visible.

---

## Target Design

One principle: **a value travels through the engine by reference once,
not by argument N times.**

### A. `EngineServices` bundle

```rust
pub struct EngineServices {
    pub llm: Arc<dyn LlmBackend>,
    pub effects: Arc<dyn EffectExecutor>,
    pub store: Arc<dyn Store>,
    pub capabilities: Arc<CapabilityRegistry>,
    pub leases: Arc<LeaseManager>,
    pub policy: Arc<PolicyEngine>,
    pub retrieval: RetrievalEngine,
    pub platform: PlatformInfo,
    pub event_tx: broadcast::Sender<ThreadEvent>,
}
```

- Constructed once in `app.rs`, held as `Arc<EngineServices>`.
- `ThreadManager`, `ExecutionLoop`, `MissionManager`, and
  `execute_orchestrator` all take `Arc<EngineServices>`.
- The `with_*` optional builders go away: production and test wire
  the same struct (tests pass an `EngineServices` built around
  `InMemoryStore` + a fake LLM).
- After: `ExecutionLoop::new(thread, services, signal_rx)`;
  `execute_orchestrator(code, thread, services, signal_rx,
  persisted_state)`.

When a manager genuinely needs only a subset, give it a narrower trait
view (`trait HasStore`, etc.) over `EngineServices` rather than going
back to per-Arc params.

### B. `ActionGateway` — single dispatch path

```rust
pub struct ActionGateway { /* leases, policy, effects, capabilities */ }

impl ActionGateway {
    pub async fn execute(&self, ctx: &StepFrame, call: ActionCall) -> ActionResult;
    pub async fn execute_batch(&self, ctx: &StepFrame, calls: Vec<ActionCall>) -> Vec<ActionResult>;
}
```

Both Tier 0 (`structured.rs`) and the Python host functions
(`orchestrator.rs::__execute_action__` / `__execute_actions_parallel__`)
delegate to `ActionGateway`. Lease consumption, policy evaluation,
capability lookup, action snapshot caching live in one place. Future
safety rules (`tool-evidence.md` empty-fast gate, the side-effect
intent gate) get one home.

This also eliminates the snapshot-passing in
`ThreadExecutionContext` — the gateway *is* the snapshot.

### C. `Thread` is the source of truth; derive `StepFrame` from it

Delete `thread_id`/`project_id`/`user_id`/`thread_type` from
`ThreadExecutionContext`. Replace its in-step usage with `&Thread`.
The remaining step-scoped fields (`step_id`, `current_call_id`,
`source_channel`, `user_timezone`) form a small `StepFrame` that lives
only inside the gateway call. No more re-deriving identity per layer.

### D. Decompose `runtime/mission.rs`

7,933 lines mixing scheduling/cron, gate evaluation, learning-mission
seeding, fire-rate limiting, budget gates, notifications. Split along
those seams — each gets a sibling file. Do this *after* (A) lands so
the constructor noise has shrunk first.

---

## Sequencing — least risky path

1. **Land `EngineServices`** as a pure refactor. Same fields, new
   home. Touches every constructor; no behavior change. Every later
   refactor is cheaper because deps stop multiplying. **Acceptance:**
   `cargo test -p ironclaw_engine` green; constructor argument count
   drops at every site by ≥4.
2. **Make `ExecutionLoop` deps required** from `EngineServices`.
   Delete `with_*` builders and the `Option<…>` checks they enable.
   **Acceptance:** zero `Option<Arc<…>>` fields on `ExecutionLoop`;
   no runtime behavior diff in trace recordings of equivalent threads.
3. **Extract `ActionGateway`.** Migrate Tier 0 first (smaller surface).
   Then port the four Python host fns to delegate. Keep behavior
   bit-identical; the existing executor tests catch drift.
   **Acceptance:** `structured.rs` no longer references `leases` or
   `policy` directly; orchestrator host fns match.
4. **Slim `ThreadExecutionContext`** to step-scoped data; introduce
   `StepFrame`. **Acceptance:** all duplicate identity fields gone;
   snapshot fields removed; orchestrator no longer re-stuffs them.
5. **Decompose `runtime/mission.rs`** along scheduling /
   gate-evaluation / learning-seed / notifications seams.
   **Acceptance:** no single file > 1500 lines in `runtime/`.

---

## Tradeoffs

`EngineServices` makes "what does this layer actually need" less
visible at the constructor — fine-grained DI traded for ergonomics.
In a crate with one process, one set of services, and no plugin
points at the manager layer, this trade is correct. The escape
hatch (subset traits) preserves the option to narrow later when a
manager genuinely diverges.

The `ActionGateway` extraction adds an indirection — but the
indirection already exists, twice, in two places. Naming it once
is a net reduction.

## Out of scope

- Public engine API surface (`lib.rs` re-exports stay).
- Trait shapes of `LlmBackend` / `Store` / `EffectExecutor`.
- The Python orchestrator code itself.
- Persistence schema.

## Related

- `crates/ironclaw_engine/CLAUDE.md` — primitives and module map.
- `docs/plans/2026-03-20-engine-v2-architecture.md` — original v2 plan.
- `.claude/rules/architecture.md` — companion rule (filed alongside
  this plan) capturing the discipline that would have prevented the
  sprawl in the first place.
