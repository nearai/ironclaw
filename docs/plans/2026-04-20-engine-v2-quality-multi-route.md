# Engine V2 Quality: Milestone 0 + Multi-Route Execution

**Date:** 2026-04-20  
**Status:** Design  
**Context:** Engine V2 currently over-relies on a single CodeAct/orchestrator path. This makes simple tasks too expensive, weakens finalization, and pushes too many fixes into prompt/orchestrator patches instead of the layer that owns the failure.

---

## Executive summary

We should **not** jump straight into a full architecture rewrite.

Instead, we should:

1. Run a **Milestone 0 orchestrator-first sprint** to test whether the current quality problems can be fixed primarily in:
   - `crates/ironclaw_engine/orchestrator/default.py`
   - CodeAct prompts
   - small supporting loop/router changes
2. If that sprint is insufficient, proceed with a target architecture built around:
   - **multi-route execution**
   - a typed **thread-level execution ledger**
   - a separate **repair planner/arbiter**
   - scoped runtime improvements at `thread`, `project`, and `tenant-agent`

This preserves the simple path if it works, while still giving us a clear next architecture if it does not.

---

## Problem

### Current symptoms

- V2 struggles with tool use and CodeAct-heavy flows
- V2 takes too long on tasks that should be straightforward
- V2 sometimes completes tool work but fails to finalize cleanly
- Current self-improvement mostly lands as prompt/orchestrator changes, even when the real problem is routing, finalization, or tool-surface behavior

### Current pressure points

1. **Ingress sets only a few execution flags**
   - Example: `require_action_attempt` is set in the router, but execution still flows through the same core path.
   - Evidence: `src/bridge/router.rs`

2. **The execution loop is still CodeAct-centric**
   - `ExecutionLoop::run()` injects the CodeAct/RLM system prompt when none exists.
   - Evidence: `crates/ironclaw_engine/src/executor/loop_engine.rs`

3. **The Python orchestrator owns many of the quality-sensitive decisions**
   - loop count
   - nudge behavior
   - finalization checks
   - repeated error handling
   - max-iteration behavior
   - Evidence: `crates/ironclaw_engine/orchestrator/default.py`

4. **Canonical control state is too implicit**
   - Important runtime state is split across:
     - `Thread.messages`
     - `Thread.internal_messages`
     - `Thread.events`
     - `Thread.metadata`
     - runtime checkpoint state
     - `ThreadOutcome`

---

## Goals

- Improve V2 completion rate
- Reduce unnecessary CodeAct churn
- Make simple tasks take simple paths
- Make completion/finalization explicit and inspectable
- Make runtime repair typed and layer-aware
- Keep the first implementation bounded
- Support `thread`, `project`, and `tenant-agent` scopes in V1
- Defer `shared/system` scope

---

## Non-goals

- Full shared/system promotion in V1
- Replacing all runtime state with one giant ledger
- Eliminating CodeAct
- Solving all cross-tenant governance concerns now
- Introducing a new persistence subsystem outside the current project/store model

---

# Milestone 0: Orchestrator-first disproof sprint

## Purpose

Before we implement the larger architecture, run a bounded experiment to determine whether the current quality issues are mostly fixable inside the existing orchestrator path.

If Milestone 0 closes most of the gap, we can reduce or delay the bigger architecture work.
If it does not, we proceed with the architecture below.

## Allowed scope

Primary surfaces:
- `crates/ironclaw_engine/orchestrator/default.py`
- `crates/ironclaw_engine/prompts/codeact_preamble.md`
- `crates/ironclaw_engine/prompts/codeact_postamble.md`

Small supporting changes allowed in:
- `crates/ironclaw_engine/src/executor/loop_engine.rs`
- `src/bridge/router.rs`

## Explicitly out of scope

- multi-route executor framework
- typed execution ledger
- repair arbiter
- tenant-agent control project
- shared/system scope

## Candidate fixes

1. **No-new-evidence cutoff**
   - Stop or escalate when recent iterations produce no meaningful new results

2. **Stronger finalization trigger**
   - Finalize when enough evidence exists instead of continuing to explore

3. **Better repeated-action-error behavior**
   - Cut off or change strategy sooner when the same tool errors repeat

4. **Simple-task bias inside current orchestrator**
   - Prefer direct tool use and shorter paths for deterministic tasks

5. **Better use of prior tool outputs**
   - Reinforce using existing results from state instead of reconstructing them

## Milestone 0 deliverables

1. Replay/eval suite for representative regressions
2. Orchestrator and prompt changes
3. Before/after quality report
4. Decision: stop here or proceed to target architecture

## Exit criteria

Milestone 0 is successful only if it materially improves:
- completion rate
- median step count
- latency
- done-but-not-finalized incidents
- repeated error loops

If not, proceed to the architecture below.

---

# Target architecture

## 1. Multi-route execution

### Routes

```rust
enum ExecutionRoute {
    Structured,
    PlannedStructured,
    CodeAct,
    Delegated,
}
```

### Intended use

- **Structured**: deterministic read/search/edit/test/respond flows
- **PlannedStructured**: short multi-step workflows with explicit checkpoints
- **CodeAct**: dynamic loops, programmatic transformations, exploratory tasks
- **Delegated**: bounded subproblems, diagnosis, parallel branches, sub-agent work

## 2. Router-owned `ExecutionPlan`

Route selection is **thread-level**, not per-step.

```rust
struct ExecutionPlan {
    route: ExecutionRoute,
    route_reason: String,
    completion_contract: CompletionContract,
    verification_policy: VerificationPolicy,
    fallback_policy: FallbackPolicy,
    repair_envelope: RepairEnvelope,
    risk_class: RiskClass,
}
```

### Ownership

- Created at ingress by the router
- Persisted on the thread ledger
- Executors may narrow/consume it
- Executors may not redefine it wholesale

## 3. Keep `ExecutionPlan` separate from `ThreadConfig`

### `ThreadConfig`
Low-level runtime guardrails:
- max iterations
- max duration
- max tokens
- consecutive error limits
- compaction settings

### `ExecutionPlan`
High-level control plane:
- route
- completion contract
- verification policy
- fallback policy
- repair envelope

## 4. Canonical `ExecutionLedger`

Add a typed minimal control-plane ledger to `Thread`.

```rust
struct ExecutionLedger {
    plan: ExecutionPlan,
    completion: CompletionState,
    obligations: Vec<ExecutionObligation>,
    evidence: Vec<EvidenceRef>,
    active_repairs: Vec<AppliedRepair>,
    pause: Option<PauseState>,
    handoffs: Vec<HandoffRecord>,
    finalization: FinalizationState,
}
```

### V1 boundary

The ledger is **minimal control-plane only**.
It does **not** absorb:
- full working messages
- full tool outputs
- route-private scratch
- full transcript state

Those remain in:
- `Thread.messages`
- `Thread.internal_messages`
- `Step`
- `ActionResult`
- route-local checkpoint state

## 5. Hybrid obligation model

Obligations should be typed for runtime and readable for models.

```rust
struct ExecutionObligation {
    kind: ObligationKind,
    status: ObligationStatus,
    description: String,
    acceptance_hint: Option<String>,
    evidence_refs: Vec<EvidenceRef>,
    required: bool,
}
```

```rust
enum ObligationKind {
    InspectTarget,
    AttemptAction,
    CollectEvidence,
    VerifyOutcome,
    ProduceUserSummary,
    ResolveGate,
    RunValidation,
}
```

## 6. Ledger-owned completion

Completion should be owned by the ledger, not inferred mainly from `ThreadOutcome`.

The ledger tracks:
- whether obligations are satisfied
- blocker/partial status
- finalizer attempts
- forced finalization
- why the thread is considered done

`ThreadOutcome` remains a transport/projection layer.

## 7. Ledger-owned pause/gate state

Pause/gate state should be canonical in the ledger.

```rust
struct PauseState {
    gate_name: String,
    action_name: String,
    call_id: String,
    resume_kind: crate::gate::ResumeKind,
    paused_at: DateTime<Utc>,
    step_id: Option<StepId>,
}
```

`ThreadOutcome::GatePaused` remains the outward projection when execution actually stops.

## 8. Separate repair planner / arbiter

Executors detect issues and propose repairs.
A separate repair planner/arbiter owns repair decisions.

```rust
enum RepairProposal {
    SwitchRoute { to: ExecutionRoute, reason: String },
    ForceFinalizer { reason: String },
    AddThreadRule { text: String, reason: String },
    ToolOverride { tool: String, reason: String, patch: ToolPatch },
    ExecutorOverride { patch: ExecutorPatch, reason: String },
}
```

## 9. Route handoff model

Route handoff is **stop + restart**, not live-switch.

1. Current executor stops with a handoff request
2. Arbiter records a `HandoffRecord`
3. Thread persists
4. New executor starts against the same thread

```rust
struct HandoffRecord {
    from: ExecutionRoute,
    to: ExecutionRoute,
    reason: String,
    at: DateTime<Utc>,
}
```

## 10. Scoped runtime improvements

### V1 scopes

```rust
enum ImprovementScope {
    Thread,
    Project,
    TenantAgent,
}
```

### Patch examples

```rust
enum ImprovementPatch {
    PromptRule { text: String },
    ToolAlias { from: String, to: String },
    ToolNormalization { tool: String, rule: String },
    ExecutorOverride { key: String, value: serde_json::Value },
    FinalizerOverride { key: String, value: serde_json::Value },
}
```

## 11. Scope and storage model

### `thread`
Stored on the thread ledger itself.

### `project`
Stored in the current project.

### `tenant-agent`
Stored in a **dedicated tenant-owned control project**.

### Deferred
`shared/system`

## 12. Tenant-agent control project

For V1, broader-than-project improvements stop at `tenant-agent`.

### Properties
- dedicated
- tenant-owned
- lazy-created
- canonical helper/factory

This avoids introducing `shared/system` before the core quality architecture proves itself.

---

# Resolved architecture decisions

- Route selection is **thread-level**
- Handoff uses a **shared canonical ledger**
- Ledger is a **typed durable field on `Thread`**
- Ledger is **minimal control-plane only**
- Completion is **ledger-owned**
- Pause/gate state is **ledger-owned**
- `ExecutionPlan` is **router-owned** and **separate from `ThreadConfig`**
- `ExecutionPlan` is **rich**, not minimal
- A separate **repair planner/arbiter** owns runtime repairs
- Handoff is **stop + restart**
- V1 scope model is **`thread + project + tenant-agent`**
- `tenant-agent` uses a **dedicated tenant-owned control project**
- Control project creation is **lazy**
- `shared/system` is **deferred**

---

# Rollout plan

## Milestone 0
- Replay/eval suite
- Orchestrator loop tightening
- Prompt tightening
- Go/no-go decision

## Phase 1
- Add core types:
  - `ExecutionRoute`
  - `ExecutionPlan`
  - `ExecutionLedger`

## Phase 2
- Add ledger-owned completion and pause state
- Add repair arbiter

## Phase 3
- Add handoff model
- Add structured route

## Phase 4
- Add scoped runtime improvements
- Add tenant-agent control project

## Phase 5
- Integrate `/expected` with typed expectation cases and route-aware diagnostics

## Phase 6
- Introduce `shared/system` scope if still justified

---

# Success metrics

Track at least:
- completion rate
- median step count
- latency
- route handoff rate
- forced finalizer rate
- repeated error loops
- done-but-not-finalized incidents
- replay pass rate

---

# Issue breakdown

## Track A — Milestone 0

### M0-1 — Build replay/eval suite for V2 quality regressions
Acceptance criteria:
- representative V1-good / V2-bad tasks captured
- metrics reported:
  - completion
  - latency
  - step count
  - finalization success
  - repeated-error loops

### M0-2 — Tighten orchestrator loop behavior in `default.py`
Scope:
- no-new-evidence cutoff
- repeated action-error handling
- stronger finalization trigger
- reduced simple-task churn

Acceptance criteria:
- targeted orchestrator changes implemented
- replay suite shows reduced looping and/or faster finalization

### M0-3 — Tighten CodeAct prompt for simple-task behavior and finalization
Scope:
- reduce unnecessary code use
- encourage direct tool usage for simple tasks
- reinforce using prior tool outputs
- strengthen finalization behavior

Acceptance criteria:
- prompt changes implemented
- replay suite shows measurable behavior improvement

### M0-4 — Evaluate Milestone 0 and decide whether to proceed
Acceptance criteria:
- before/after metrics documented
- explicit go/no-go decision recorded
- if insufficient improvement, architecture track proceeds unchanged

## Track B — Architecture follow-on

### B1 — Add core engine types: `ExecutionRoute`, `ExecutionPlan`, `ExecutionLedger`
### B2 — Split `ExecutionPlan` from `ThreadConfig`
### B3 — Add hybrid obligation and completion state to the ledger
### B4 — Move pause/gate state into the ledger
### B5 — Introduce repair planner/arbiter and typed repair proposals
### B6 — Implement route handoff as stop + restart on same thread
### B7 — Add router-owned multi-route `ExecutionPlan` selection
### B8 — Retrofit CodeAct executor to consume `ExecutionPlan` + `ExecutionLedger`
### B9 — Implement initial structured route and shared handoff surface
### B10 — Add scoped runtime improvement patches for `thread`, `project`, and `tenant-agent`
### B11 — Add dedicated tenant-agent control project with lazy canonical creation
### B12 — Integrate `/expected` with typed expectation cases and route-aware diagnostics
### B13 — Add finalizer stage and done-but-unfinalized recovery
### B14 — Add telemetry, replay evals, and phased rollout controls

---

## Recommended immediate order

### Right now
1. M0-1
2. M0-2
3. M0-3
4. M0-4

### Then, if needed
1. B1
2. B2
3. B3
4. B4
