# Reborn Agent Loop — Skeleton Framework

**Date:** 2026-05-12
**Status:** Architecture spec for the skeleton-framework PR
**Builds on:** [`turns-agent-loop.md`](contracts/turns-agent-loop.md), [`lightweight-agent-loop.md`](contracts/lightweight-agent-loop.md), [`loop-exit.md`](contracts/loop-exit.md), [`turn-runner.md`](contracts/turn-runner.md), [`2026-05-12-agent-loop-context-model-draft.md`](2026-05-12-agent-loop-context-model-draft.md)
**Implementation briefs:** [`agent-loop-briefs/`](agent-loop-briefs/)

---

## 1. Purpose

This document is the canonical architecture spec for the Reborn agent-loop skeleton framework — a new crate (`ironclaw_agent_loop`) that adds a reusable loop body and a strategy-composition planner above the existing `TurnCoordinator → TurnRunner → AgentLoopDriver → AgentLoopHost` chain.

The skeleton ships **trait scaffolding plus this design doc**. No tool-capable driver, no real `LoopCapabilityPort` wiring; those land in follow-up PRs once the framework contract is locked. Nine per-workstream implementation briefs (under [`agent-loop-briefs/`](agent-loop-briefs/)) carve the work into independently committable pieces, with WS-8 owning the cross-workstream integration suite that proves the framework actually composes into a working loop.

The default behavior models the pi-mono agent loop mechanics — kept simple and well-understood. The framework itself is loop-family-agnostic.

## 2. Why this exists

Today the engine has:

- A single `AgentLoopDriver` trait (`crates/ironclaw_turns/src/run_profile/driver.rs:85`) with `run`/`resume` methods.
- One concrete driver, `TextOnlyModelReplyDriver` (`crates/ironclaw_reborn/src/text_loop_driver.rs`), that bakes the entire loop — prompt build, model call, reply finalize — into one impl block.
- `EmptyLoopCapabilityPort` stubbing the capability surface.

That works for the text-only first slice but gives no shared body for future loop families (routines, missions, general assistant, coding, planning). Each new family today means writing another 200-line driver from scratch, duplicating the tick mechanics.

The skeleton fixes that by separating three concerns that the current driver conflates:

| Concern | Owns | Belongs in |
|---|---|---|
| **Loop strategy** ("what should this loop family do at each decision point?") | `AgentLoopPlanner` (composition of nine strategies) | `ironclaw_agent_loop` |
| **Loop mechanics** ("the canonical tick") | `AgentLoopExecutor` | `ironclaw_agent_loop` |
| **Runner adapter** ("turn the framework into something the runner can call") | `PlannedDriver<P, E>: AgentLoopDriver` | `ironclaw_reborn` |

Each loop family then becomes "pick nine strategies" — usually overriding two or three from the defaults — instead of writing a new driver.

## 3. Architecture overview

```text
TurnCoordinator                schedules / queues / one-active-run gate
      ↓
TurnRunner                     claims a run, builds the host facade,
                               looks up an AgentLoopDriver, invokes it,
                               validates LoopExit, persists transition
      ↓
AgentLoopDriver  (trait)       runner-facing boundary  ← lives in ironclaw_turns
      ↓
PlannedDriver<P, E>            adapter implementing AgentLoopDriver
                               over (planner: P, executor: E)         ← lives in ironclaw_reborn
      ↓
AgentLoopExecutor              canonical loop tick                    ──┐
      ↓                                                                  │
AgentLoopPlanner               composition of nine strategies            ├── lives in ironclaw_agent_loop
      ↓                                                                  │
nine Strategy traits           one decision per trait                    │
      ↓                                                                  │
AgentLoopDriverHost            host ports the executor calls           ──┘ ← trait lives in ironclaw_turns
      ↓                          (model, prompt, capability, transcript,
                                  checkpoint, progress, input)
      ↓
host backends                  durable transcript, checkpoint store,
                               event log, model gateway, capability host
```

The framework crate (`ironclaw_agent_loop`) does *not* know about the runner-facing `AgentLoopDriver` trait. It owns "what a loop is." `PlannedDriver` in `ironclaw_reborn` is the only thing that bridges the two.

## 4. Crate layout

```text
ironclaw_turns                              (unchanged surface; one new variant)
  src/run_profile/
    driver.rs                  AgentLoopDriver trait, descriptor                   (existing)
    host.rs                    AgentLoopDriverHost, LoopRunContext,                (existing)
                               all LoopXxxPort traits, LoopModelRouteSnapshot
    refs.rs                    LoopMessageRef / LoopResultRef / etc.               (existing)
  src/loop_exit.rs             LoopExit + variants                                 (gains LoopFailureKind::NoProgressDetected)
  src/runner.rs                TurnRunner interfaces                               (existing)
  src/coordinator.rs           TurnCoordinator                                     (existing)

ironclaw_agent_loop                         NEW — owns "what a loop is"
  src/lib.rs
  src/state.rs                 LoopExecutionState (immutable) + per-strategy slots
                               + BoundedRing<T, N> + CapabilityCallSignature
  src/strategies/
    mod.rs                     exports
    context.rs                 ContextStrategy trait + DefaultContextStrategy
    capability.rs              CapabilityStrategy trait + DefaultCapabilityStrategy
    model.rs                   ModelStrategy trait + DefaultModelStrategy
    batch.rs                   BatchPolicyStrategy trait + DefaultBatchPolicyStrategy
    gate.rs                    GateHandlingStrategy trait + GateOutcome + DefaultGateHandlingStrategy
    recovery.rs                RecoveryStrategy trait + RecoveryOutcome + DefaultRecoveryStrategy
    stop.rs                    StopConditionStrategy trait + StopOutcome + DefaultStopConditionStrategy
    drain.rs                   InputDrainStrategy trait + DefaultInputDrainStrategy
    budget.rs                  BudgetStrategy trait + DefaultBudgetStrategy
  src/planner.rs               AgentLoopPlanner facade trait
  src/executor.rs              AgentLoopExecutor trait + canonical-tick contract
  src/canonical_executor.rs    CanonicalAgentLoopExecutor (default impl)
  src/default_planner.rs       DefaultPlanner with nine Default* slots; impl Default

ironclaw_reborn                             (tighter — runtime integration)
  src/text_loop_driver.rs      TextOnlyModelReplyDriver (existing, unchanged)
  src/planned_driver.rs        NEW — PlannedDriver<P, E> implements AgentLoopDriver
  src/turn_runner.rs           registers PlannedDriver instances by id            (existing)
  src/driver_registry.rs       (existing)
  src/loop_exit_applier.rs     (existing)
```

Each follow-up loop family ships as one factory function in `ironclaw_agent_loop/src/families/<name>.rs`, e.g. `coding_planner()` returning a `DefaultPlanner` with select strategies swapped. The skeleton ships none of these; only `DefaultPlanner::default()`. A family graduates to its own crate only when it pulls heavyweight external deps (tree-sitter, ripgrep, etc.).

## 5. Mutability layers

Four distinct things in the loop world; each has different mutability and different ownership. Briefs must respect this layering.

| Layer | Type | Mutability | Who mutates | Crate |
|---|---|---|---|---|
| 1 | `LoopRunContext` | **Immutable** for the entire claimed run (and across resume) | `TurnRunner` writes once at claim time; never again | `ironclaw_turns` |
| 2 | `LoopExecutionState` | **Value-immutable**; the executor's local `let mut state` is rebound each tick to the next whole state | Strategies return new own-slot values; executor builds the next whole state by swapping slots | `ironclaw_agent_loop` |
| 3 | `TurnRunState` | **Mutable** lifecycle (`accepted → queued → running → blocked_* → completed/failed/cancelled`) | `TurnRunner` only — not the loop, not the executor, not strategies | `ironclaw_turns` |
| 4 | Host-managed durable state (transcript, checkpoint store, event log) | **Mutable** via host port calls | Loop *requests* writes through `LoopTranscriptPort` / `LoopCheckpointPort` / `LoopProgressPort`; the host owns the actual mutation | host backends |

The loop:

- **reads** layer 1 via `host.run_context()`
- **threads** layer 2 through itself, rebinding each tick
- **never directly touches** layer 3 — it returns `LoopExit`; the runner translates that into a durable `TurnRunState` transition
- **requests** layer 4 mutations through host ports

There is no `state.set_completed()`-style API on the loop side. The loop returns `LoopExit`; `LoopExitApplier` (in `ironclaw_reborn`) validates the refs in the exit and applies the durable transition. This is what makes evidence validation possible.

## 6. The nine strategies

Each strategy is one small Rust trait with one or two methods. Default impls model pi-mono behavior. A loop family typically swaps two or three of them; the rest stay default.

| Strategy | Decision it owns | Returns | Default behavior |
|---|---|---|---|
| `ContextStrategy` | What prompt mode + sections + optional inline messages to request | `LoopPromptBundleRequest` | `PromptMode::TextOnly`, all standard sections, no inline message, max 16 messages |
| `CapabilityStrategy` | Which capabilities are visible this iteration | `CapabilityFilter` | All allowed; expect provider-tool encoding |
| `ModelStrategy` | Which model preference to ask the host for | `ModelPreference` | Primary route only |
| `BatchPolicyStrategy` | Sequential vs parallel for a capability batch | `BatchPolicy` | Parallel for read-only; sequential for writes |
| `GateHandlingStrategy` | On Approval/Auth/Resource gate: block/skip/abort | `GateOutcome` (mutates `control_state`) | Always block (checkpoint + return `LoopExit::Blocked`) |
| `RecoveryStrategy` | On capability/model error: retry/skip/abort | `RecoveryOutcome` (mutates `recovery_state`) | Retry transient model errors 2× with backoff |
| `StopConditionStrategy` | Should we stop after this completed turn? | `StopOutcome` (mutates `control_state`) | Stop on terminate-hint; no-progress detection (see §10) |
| `InputDrainStrategy` | When to drain steering / followup queues | `(drain_steering: bool, drain_followup: bool)` | Steering before each model call; followup only when otherwise stopping |
| `BudgetStrategy` | Iteration / wall-clock limits | `IterationLimit` (+ `Option<Duration>`) | 32 iterations, no wall-clock cap |

Only `Recovery`, `Stop`, and `Gate` mutate per-strategy state and therefore return outcome enums. The other six are pure policy and return their value directly.

Inline messages — the role pi's nudge mechanism plays — are produced by `ContextStrategy` returning a `LoopPromptBundleRequest` with an `inline_messages` field. There is no separate `NudgeStrategy`; nudges are loop-family-specific context shaping.

## 7. State model

```rust
pub struct LoopExecutionState {
    // executor-universal
    pub iteration: u32,
    pub last_checkpoint: Option<CheckpointMarker>,
    pub assistant_refs: Vec<LoopMessageRef>,
    pub result_refs: Vec<LoopResultRef>,
    pub last_gate: Option<LoopGateRef>,
    pub input_cursor: LoopInputCursor,
    pub surface_version: Option<VisibleSurfaceVersion>,

    // executor-observed (populated by the executor as calls/errors go by;
    // read-only to strategies — used for repetition / no-progress detection)
    pub recent_call_signatures: BoundedRing<CapabilityCallSignature, 8>,
    pub recent_failure_kinds:   BoundedRing<LoopFailureKind, 8>,

    // strategy slots (one per strategy that needs persistent state)
    pub context_state:    ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state:      ModelStrategyState,    // current fallback chain index (skeleton: always 0)
    pub recovery_state:   RecoveryStrategyState, // attempt counters
    pub control_state:    ControlStrategyState,  // milestones, terminate-hints seen, gate fingerprints
}
```

`BoundedRing<T, N>` is a small fixed-capacity ring buffer with helpers:

- `push(item: T)` — drops oldest at capacity
- `most_common_count_in(window: usize) -> usize`
- `same_run_length() -> usize`

`CapabilityCallSignature` is `(CapabilityName, ArgsHash)` — a stable hash over the capability name plus canonicalized JSON args. Lets the executor cheaply detect "same call repeated" without retaining the args themselves (no raw tool input in state, per [`turns-agent-loop.md`](contracts/turns-agent-loop.md) §6).

Strategy outcome shape (example for `RecoveryStrategy`):

```rust
pub enum RecoveryOutcome {
    Retry      { recovery: RecoveryStrategyState, alter: Option<RetryAlteration> },
    SkipResult { recovery: RecoveryStrategyState },
    Abort      { recovery: RecoveryStrategyState, failure_kind: LoopFailureKind },
}
```

The strategy returns the new value of *its own slot only*. The executor builds the next whole state by swapping that slot. The compiler enforces that `RecoveryStrategy` cannot rewrite `BudgetStrategyState`.

## 8. The canonical executor tick

Pseudocode of `CanonicalAgentLoopExecutor::execute`:

```text
state = LoopExecutionState::initial(host.run_context())  // OR ::from_checkpoint on resume

loop:
  // 0. Iteration cap at TOP of loop (not bottom). Resume with state.iteration
  //    already at limit must exit immediately, not run one more body.
  if state.iteration >= planner.budget().iteration_limit(&state):
    return LoopExit::Failed { reason_kind: IterationLimit, ... }

  // 1. Cancellation observation — checkpoint + Ok(LoopExit::Cancelled(...)) if fired.
  observe_cancellation_then_checkpoint_and_exit_if_set()

  // 2. Steering drain. LoopInputPort surface is poll_inputs(after, limit) +
  //    ack_inputs(cursor). Filter to user-facing kinds only — control kinds
  //    (Cancel, Interrupt, GateResolved, CapabilitySurfaceChanged) are NOT
  //    consumed here.
  if planner.drain().drain_steering(&state):
    pending = host.poll_inputs(state.input_cursor, MAX_PER_DRAIN)
    (steering_msgs, last_consumed) = filter_steering_kinds(pending)
    if !steering_msgs.is_empty():
      state.append_inputs(steering_msgs)
      host.ack_inputs(last_consumed)
      state.input_cursor = last_consumed

  ctx_req   = planner.context().plan_context_request(&state)
  bundle    = host.build_prompt_bundle(ctx_req)
  surface   = host.visible_capabilities(planner.capability().filter(&state))
  state.surface_version = Some(surface.version)

  checkpoint(BeforeModel, &state)

  model_pref = planner.model().preference(&state)
  model_resp = host.stream_model(LoopModelRequest { messages: bundle.messages,
                                                   surface_version: surface.version,
                                                   model_preference: Some(model_pref) })

  match model_resp.output:
    ParentLoopOutput::AssistantReply(reply):
      // Finalize FIRST, before stop-condition branching, so EVERY exit path
      // (Completed or Failed) carries the assistant ref. LoopExit validation
      // rejects a non-NoReply Completed without reply_message_refs.
      reply_ref = host.finalize_assistant_message(FinalizeAssistantMessage { reply })
      state.assistant_refs.push(reply_ref.clone())

      summary = TurnSummary { kind: ReplyOnly, assistant_message_ref: Some(reply_ref) }
      stop = planner.stop().should_stop_after_turn(&state, &summary)
      match stop:
        Stop { control, GracefulStop }:
          state.control_state = control
          checkpoint(Final, &state)
          return LoopExit::Completed { reply_message_refs: state.assistant_refs.clone(), ... }
        Stop { control, NoProgressDetected }:
          state.control_state = control
          checkpoint(Final, &state)
          return LoopExit::Failed { reason_kind: NoProgressDetected, ... }
        Stop { control, Aborted(fk) }:
          state.control_state = control
          return LoopExit::Failed { reason_kind: fk, ... }
        Continue { control }:
          state.control_state = control
          // Followup drain: even on Continue→Completed, the reply ref is
          // already finalized and in state.assistant_refs.
          if planner.drain().drain_followup(&state):
            (state, drained) = drain_followup_into(state)
            if !drained:
              checkpoint(Final, &state)
              return LoopExit::Completed { reply_message_refs: state.assistant_refs.clone(), ... }
          else:
            checkpoint(Final, &state)
            return LoopExit::Completed { reply_message_refs: state.assistant_refs.clone(), ... }

    ParentLoopOutput::CapabilityCalls(calls):
      checkpoint(BeforeSideEffect, &state)
      result_refs_start = state.result_refs.len()  // snapshot for batch summary
      policy   = planner.batch().policy(&state, &calls.summaries(&surface))
      outcomes = host.invoke_capability_batch(calls, policy)
      iteration_signatures = HashSet::new()  // per-iteration dedupe (§10 + WS-0 §3.4)
      for (call, outcome) in calls.zip(outcomes):
        sig = signature_of(call)
        if iteration_signatures.insert(sig.clone()):
          state.recent_call_signatures.push(sig)
        match outcome:
          Completed(result):
            state.append_result(result)
          ApprovalRequired(g) | AuthRequired(g) | ResourceBlocked(g):
            // Gate handling — Block/SkipAndContinue/Abort per planner.gate().
            // (See WS-6 §3.3 for full match.)
          Denied(reason):
            // EmptyLoopCapabilityPort returns Denied; capability policy can
            // also deny at any time. Treat as a non-recoverable failure for
            // THIS call; consult Recovery to skip-and-continue or abort batch.
          SpawnedProcess(handle):
            // Long-running async process. Checkpoint + return Blocked; resume
            // when the process emits its completion event via LoopInputPort.
            state.last_gate = Some(handle.gate_ref())
            checkpoint(BeforeBlock, &state)
            return LoopExit::Blocked { kind: ResourceWaitingForProcess, ... }
          Failed(err):
            // Push failure kind ONCE per call (not per retry attempt) —
            // otherwise three retries of one call would falsely satisfy
            // failure-run-length detection.
            state.recent_failure_kinds.push(err.kind)
            loop:
              recovery = planner.recovery().on_capability_error(&state, &err.summary)
              match recovery:
                Retry { recovery, alter }:
                  state.recovery_state = recovery
                  honor_alteration(alter)
                  retry_outcome = host.invoke_capability(call)
                  match retry_outcome:
                    Completed(result): state.append_result(result); break
                    Failed(next_err):  err = next_err; continue  // do NOT re-push kind
                    other:             promote to outer arm via helper
                SkipResult { recovery }: state.recovery_state = recovery; break
                Abort { recovery, fk }:  return LoopExit::Failed { reason_kind: fk, ... }

      // Post-batch stop check — slice exactly THIS batch's refs from the
      // snapshot index (not by call count, which would over-include refs
      // from prior iterations on Skip/Block/Failed-with-no-retry batches).
      summary = TurnSummary {
        kind: AfterCapabilityBatch,
        batch_result_refs: state.result_refs[result_refs_start..].to_vec(),
      }
      stop = planner.stop().should_stop_after_turn(&state, &summary)
      match stop:
        Stop { GracefulStop }:    checkpoint(Final, &state); return LoopExit::Completed { ... }
        Stop { NoProgressDetected }: checkpoint(Final, &state); return LoopExit::Failed { NoProgressDetected, ... }
        Stop { Aborted(fk) }:     return LoopExit::Failed { reason_kind: fk, ... }
        Continue { control }:     state.control_state = control  // fall through

  state.iteration += 1   // increment for next iteration's top-of-loop budget check
```

Three properties the canonical executor must guarantee, regardless of strategy choices:

1. **Checkpoint discipline** — checkpoints land at the four boundary kinds (`BeforeModel`, `BeforeSideEffect`, `BeforeBlock`, optionally `Final`) and nowhere else. Strategies cannot trigger checkpoints.
2. **Cancellation observation** — checked between every strategy call. On cancel: checkpoint current state, return `Ok(LoopExit::Cancelled(...))` — cancellation is a successful exit, not an executor error. (`AgentLoopExecutorError::Cancelled` is reserved for the unrecoverable edge case where the executor cannot even produce a `LoopExit::Cancelled`.)
3. **Single mutation point** — `state` is rebound in exactly one place per branch. No interior mutability, no `&mut` across strategy calls.

## 9. Cross-cutting decisions (locked)

- **Checkpoint discipline is executor-owned.** Four kinds: `BeforeModel`, `BeforeSideEffect`, `BeforeBlock`, optionally `Final`. Strategies cannot trigger checkpoints; they only return state slots.
- **Cancellation observed between strategy calls.** Strategies never see the signal directly.
- **Visible surface version pinned per iteration** before `plan_model_request`, held in `LoopExecutionState.surface_version`. On stale-surface outcome, executor reloads + retries that iteration; counts against `BudgetStrategy.iteration_limit(&state)`.
- **Error sanitization at the host boundary.** Strategies receive `CapabilityErrorSummary` / `ModelErrorSummary` (already redacted by the host). Raw provider errors never reach planner code. Honors [`error-handling.md`](../../.claude/rules/error-handling.md) channel-edge rule.
- **Fallback chain is intended but deferred.** Skeleton keeps the existing `Option<LoopModelRouteSnapshot>` on `LoopRunContext` and reserves `model_state.fallback_index: u32` (always 0 in skeleton). When a future `RecoveryStrategy` needs to switch models, that PR adds `ModelRouteChain` to `host.rs` and migrates the storage layer call sites. Until then, `RecoveryOutcome::Retry { alter }` cannot include a model-route swap — only context/prompt-shape alterations.
- **Async only where genuinely needed.** Pure-policy strategies (`BudgetStrategy`, `BatchPolicyStrategy`) are sync `fn`. Strategies that may consult host state (recovery, gate handling, drain) are async.
- **Production-safe escape by default** (see §10).
- **Message projection stays host-side** (`LoopPromptPort`). No `MessageProjectionStrategy` in the framework.
- **Loop families are factory functions** in `ironclaw_agent_loop/src/families/`. Single-crate model unless a family pulls heavyweight deps.
- **Naming convention: `Default*` for default impls.** No "pi" in identifiers.
- **Term: `Strategy`** for sub-components of the planner facade.
- **`AgentLoopDriver` trait is the boundary** between `ironclaw_reborn` and the framework. The framework crate does not depend on `AgentLoopDriver`.

## 10. Production-safe escape

The `Default*` strategies provide three independent stuck-loop safety nets, layered:

1. **Iteration cap.** `DefaultBudgetStrategy.iteration_limit(&state)` returns `32`. Hard ceiling. Returns `LoopExit::Failed { reason_kind: IterationLimit }`.
2. **Per-error retry budget.** `DefaultRecoveryStrategy` aborts after 2 retries on a single error class. Returns `LoopExit::Failed { reason_kind: <originating-class> }`.
3. **Repetition / no-progress escape.** `DefaultStopConditionStrategy` returns `Stop { kind: NoProgressDetected }` if either:
   - the same `CapabilityCallSignature` is observed in ≥3 of the last 5 iterations, OR
   - the same `LoopFailureKind` appears ≥3 times in a row.

The "iterations" count is critical: a single iteration containing three identical calls in one batch counts as **one** observation, not three. The executor enforces this by deduplicating signature pushes within each iteration (see WS-0 §3.4 "per-iteration push semantics"). Retries of the same call within an iteration also do not re-push. This prevents a single fan-out batch from spuriously tripping the detector while still catching genuine cross-iteration loops.

The `LoopFailureKind::NoProgressDetected` variant is added in `ironclaw_turns::loop_exit` under WS-0.

Loop families that legitimately repeat (e.g. routines polling the same capability on schedule) opt out by swapping `StopConditionStrategy` for one that ignores the signature ring.

## 11. What this skeleton is not

The skeleton (WS-0..WS-8) ships the framework crate, traits, default strategies, executor, driver adapter, and integration tests. It deliberately does NOT ship the host-port wiring, capability execution, durable persistence, or driver registration that an end-to-end agent loop needs. Those land as the follow-up workstreams documented in §12.

- **Not a tool-capable driver runtime.** `PlannedDriver(DefaultPlanner, CanonicalExecutor)` is itself tool-capable, but it can only execute capabilities once `LoopCapabilityPort` is wired (WS-9). Until then, capability calls still hit `EmptyLoopCapabilityPort` and fail closed.
- **Not a checkpoint backing store** (WS-10). The schema id `reborn:default-loop-v1` is reserved by WS-0; the producer is the follow-up.
- **Not a `LoopInputPort` implementation** (WS-11). Steering/followup queues stay non-functional.
- **Not a `LoopProgressPort` implementation** (WS-12). Executor milestone emission is no-op until wired.
- **Not a cancellation accessor on the host** (WS-13). The executor's cancellation-observation point in WS-6 §3.5 is documented but the host method it calls doesn't exist yet.
- **Not driver registration or run-profile selection** (WS-14). Submitted turns still resolve to the existing `TextOnlyModelReplyDriver` until the registry + resolver land.
- **Not a migration of `TextOnlyModelReplyDriver`.** Existing driver stays as-is until tool-capable driver work makes the migration worthwhile.
- **Not a `prepareNextTurn`-style mid-run model swap** beyond the (deferred) fallback chain mechanism (`ModelRouteChain`).
- **Not a `MessageProjectionStrategy`.** Host owns projection.
- **Not a `NudgeStrategy`.** Inline messages flow through `ContextStrategy`.
- **Not loop-family factories.** Skeleton ships `DefaultPlanner::default()` only.
- **Not an `AgentLoopPlannerDescriptor` separate type.** `PlannerId` newtype in checkpoint payload metadata is enough; richer descriptors live on the driver side via `AgentLoopDriverDescriptor`.

## 12. Follow-up workstreams for end-to-end execution

These workstreams convert the skeleton from "framework that compiles and tests against mocks" to "agent loop that actually runs end-to-end against the host runtime." Each one is independently scopable; together they close every gap in §11 between the skeleton and a working tool-using loop.

| ID | Title | Crates | Unblocks |
|----|-------|--------|----------|
| WS-9 | LoopCapabilityPort wired to host runtime | `ironclaw_loop_support` + `ironclaw_reborn` | Replace `EmptyLoopCapabilityPort` with a host-runtime impl that routes both `invoke_capability` and `invoke_capability_batch` through `CapabilityHost` with action-time auth. Capability calls actually execute. |
| WS-10 | Checkpoint store + resume path | `ironclaw_turns` (trait extension) + a checkpoint store crate | Add `load_checkpoint_payload(checkpoint_id) -> Vec<u8>` to `LoopCheckpointPort`. Implement durable backing store for `checkpoint` writes. Wire `PlannedDriver::resume` against the load path. Resume from `Blocked` actually works. |
| WS-11 | LoopInputPort implementation | `ironclaw_loop_support` | Implement `poll_inputs` + `ack_inputs` against host steering/followup queues. Steering messages reach the model mid-loop; followup messages restart the loop after a natural stop. |
| WS-12 | LoopProgressPort wiring | `ironclaw_loop_support` + `ironclaw_reborn` | Implement `emit_loop_progress` against the engine event substrate (per [`contracts/events.md`](contracts/events.md)). Executor emits milestones at strategy boundaries; SSE/audit observers see loop progress. |
| WS-13 | Cancellation accessor on AgentLoopDriverHost | `ironclaw_turns` (trait extension) + `ironclaw_loop_support` | Add the cancellation-observation method WS-6 §3.5 documents but never lands. Without this the executor cannot honor mid-loop cancellation. |
| WS-14 | PlannedDriver registration + run-profile selection | `ironclaw_reborn` | Register `default_planned_driver()` in `DriverRegistry` under a stable `LoopDriverId`. Update run-profile resolver so a profile can select `PlannedDriver` instead of `TextOnlyModelReplyDriver`. First end-to-end run through the new framework. |

**Minimum E2E set:** WS-9 + WS-13 + WS-14 are the smallest combination that lets a real turn run through the framework end-to-end (capability execution + cancellation + driver routing). WS-10 adds resume; WS-11 adds steering UX; WS-12 adds observability.

**Sequencing:** all six follow-ups can run in parallel after the skeleton (WS-0..WS-8) lands. None depend on each other directly. WS-9 and WS-13 unblock the integration smoke tests against the *real* host (not just `MockAgentLoopDriverHost`).

**Crate ownership rule for follow-ups:**
- **Trait extensions** (new methods on `LoopXxxPort`, new variants on `LoopFailureKind`, etc.) live in `ironclaw_turns`. The contracts crate is the single source of truth for runner-facing API shape.
- **Host-runtime adapters** (concrete `LoopXxxPort` impls that consult host backends) live in `ironclaw_loop_support`. Today this houses `ThreadBackedLoopContextPort`, `ThreadBackedLoopTranscriptPort`, `ThreadBackedLoopModelPort`, `EmptyLoopCapabilityPort`. WS-9, WS-11, WS-12 land their impls here.
- **Driver-side integration** (registry wiring, `LoopExitApplier`, `PlannedDriver` registration, run-profile resolution) lives in `ironclaw_reborn`. WS-13 (cancellation) splits: trait method to `turns`, accessor wiring to `loop_support`. WS-14 (registration + resolver) is fully in `reborn`.

This rule disambiguates the "loop_support + reborn" hedges in the table above: each row's primary owner is `loop_support` for the impl, with the `reborn` portion limited to wiring the impl into `AgentLoopDriverHost` composition / registry registration.

**Deferred-not-required:** `ModelRouteChain` (master doc §9), loop-family planners (`coding_planner()`, `routine_planner(...)`, etc.), and migration of `TextOnlyModelReplyDriver` are useful but not on the E2E critical path. Ship them when there's a concrete consumer.

Briefs for these follow-ups will land under [`agent-loop-briefs/`](agent-loop-briefs/) with filenames matching their workstream titles. They are intentionally not pre-written here — each should be scoped against the actual code state at the time it's picked up, not the skeleton's snapshot.

## 13. Workstream map

Nine implementation briefs live in [`agent-loop-briefs/`](agent-loop-briefs/). Briefs run in parallel within a layer; dependency edges shown.

| ID | Brief | Crate(s) | Depends on |
|----|-------|----------|------------|
| WS-0 | [`state-and-checkpoints.md`](agent-loop-briefs/state-and-checkpoints.md) — `LoopExecutionState`, slots, `BoundedRing`, `CapabilityCallSignature`, checkpoint payload schema, `LoopFailureKind::NoProgressDetected` | `ironclaw_agent_loop` + `ironclaw_turns` | — |
| WS-1 | [`strategy-traits-alpha.md`](agent-loop-briefs/strategy-traits-alpha.md) — `ContextStrategy`, `CapabilityStrategy`, `ModelStrategy` | `ironclaw_agent_loop` | WS-0 |
| WS-2 | [`strategy-traits-beta.md`](agent-loop-briefs/strategy-traits-beta.md) — `BatchPolicyStrategy`, `GateHandlingStrategy`, `RecoveryStrategy` | `ironclaw_agent_loop` | WS-0 |
| WS-3 | [`strategy-traits-gamma.md`](agent-loop-briefs/strategy-traits-gamma.md) — `StopConditionStrategy`, `InputDrainStrategy`, `BudgetStrategy` | `ironclaw_agent_loop` | WS-0 |
| WS-4 | [`planner-facade.md`](agent-loop-briefs/planner-facade.md) — `AgentLoopPlanner` + `DefaultPlanner` skeleton | `ironclaw_agent_loop` | WS-1, WS-2, WS-3 |
| WS-5 | [`default-strategies.md`](agent-loop-briefs/default-strategies.md) — nine `Default*Strategy` impls | `ironclaw_agent_loop` | WS-1, WS-2, WS-3 |
| WS-6 | [`canonical-executor.md`](agent-loop-briefs/canonical-executor.md) — `AgentLoopExecutor` + `CanonicalAgentLoopExecutor` | `ironclaw_agent_loop` | WS-4, WS-5 |
| WS-7 | [`planned-driver-adapter.md`](agent-loop-briefs/planned-driver-adapter.md) — `PlannedDriver<P, E>` adapter + registry wiring | `ironclaw_reborn` | WS-6 |
| WS-8 | [`e2e-integration-tests.md`](agent-loop-briefs/e2e-integration-tests.md) — feature-gated `test_support` module + cross-crate integration suite (happy paths, safety nets, strategy intersections, state lifecycle, reborn-side driver e2e) | `ironclaw_agent_loop` + `ironclaw_reborn` | WS-7 |

Realistic parallelism: WS-0 ships first; then WS-1/2/3 land in parallel; then WS-4/5 in parallel; then WS-6; then WS-7; then WS-8 closes the suite. WS-8 is the proof-of-life — `cargo test --workspace --features ironclaw_agent_loop/test-support` only goes green when every prior workstream is correctly composed.

## 14. Glossary

- **Driver** — runner-facing trait `AgentLoopDriver` (`ironclaw_turns`). Single job: the contract `TurnRunner` calls. Implementations either bake a whole loop (legacy `TextOnlyModelReplyDriver`) or adapt the framework (`PlannedDriver`).
- **Planner** — `AgentLoopPlanner`, the composition of nine strategies that defines a loop family.
- **Executor** — `AgentLoopExecutor`, the canonical tick body.
- **Strategy** — one swappable decision-procedure consulted by the executor at a specific point in the tick.
- **State** — `LoopExecutionState`, value-immutable, rebound per tick.
- **Run context** — `LoopRunContext`, immutable for the entire claimed run.
- **Loop family** — a particular composition of strategies, packaged as a factory function. Skeleton ships only `DefaultPlanner::default()`.

## 15. Credits

The default loop mechanics — single async function, `Reply | CapabilityCalls` parent protocol, steering/follow-up queue ergonomics — are modeled on the [pi-mono](https://github.com/badlogic/pi-mono) `packages/agent` loop. Reborn's framework absorbs pi's hooks into typed ports (`LoopPromptPort`, `LoopCapabilityPort`, `LoopInputPort`) and adds production-grade safety nets (no-progress detection, retry budgets, gate suspension, evidence-validated `LoopExit`) that pi-mono's local-developer model doesn't need.
