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
| **Runner adapter** ("turn the framework into something the runner can call") | `PlannedDriver: AgentLoopDriver` (non-generic; holds `Arc<LoopFamily>`) | `ironclaw_reborn` |

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
PlannedDriver                  non-generic adapter implementing AgentLoopDriver;
                               holds Arc<LoopFamily> + Arc<CanonicalAgentLoopExecutor>
                                                                       ← lives in ironclaw_reborn
      ↓
AgentLoopExecutor              canonical loop tick (entry: execute_family) ──┐
      ↓                                                                       │
LoopFamily                     Builtin, sealed; opaque to ironclaw_reborn     ├── lives in ironclaw_agent_loop
      ↓                                                                       │
AgentLoopPlanner               composition of nine strategies (pub trait,     │
                               sealed; strategy access via pub(crate) trait)  │
      ↓                                                                       │
nine Strategy traits           pub(crate) — one decision per trait            │
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
  src/loop_exit.rs             LoopExit + variants                                 (gains LoopFailureKind::NoProgressDetected + ::PolicyDenied)
  src/runner.rs                TurnRunner interfaces                               (existing)
  src/coordinator.rs           TurnCoordinator                                     (existing)

ironclaw_agent_loop                         NEW — owns "what a loop is"
  src/lib.rs
  src/family.rs                LoopFamilyId, ComponentIdentity, LoopFamily,
                               LoopFamilyRegistry (Builtin-only, sealed)
  src/families/mod.rs          families::default() factory; future families add here
  src/state.rs                 LoopExecutionState (immutable) + per-strategy slots
                               (StopStrategyState, GateStrategyState, …)
                               + BoundedRing<T, N> + CapabilityCallSignature
  src/strategies/              ← all traits below are pub(crate); strategies are
    mod.rs                       Builtin-only, never part of the public surface
    context.rs                 ContextStrategy trait + DefaultContextStrategy
    capability.rs              CapabilityStrategy trait + DefaultCapabilityStrategy
    model.rs                   ModelStrategy trait + DefaultModelStrategy
    batch.rs                   BatchPolicyStrategy trait + DefaultBatchPolicyStrategy
    gate.rs                    GateHandlingStrategy trait + GateOutcome + DefaultGateHandlingStrategy
    recovery.rs                RecoveryStrategy trait + RecoveryOutcome + DefaultRecoveryStrategy
    stop.rs                    StopConditionStrategy trait + StopOutcome + DefaultStopConditionStrategy
    drain.rs                   InputDrainStrategy trait + DefaultInputDrainStrategy
    budget.rs                  BudgetStrategy trait + DefaultBudgetStrategy
  src/planner.rs               AgentLoopPlanner pub trait (sealed) +
                               pub(crate) AgentLoopPlannerInternal extension trait
  src/executor.rs              AgentLoopExecutor trait + canonical-tick contract
                               (entry point: execute_family(&LoopFamily, …))
  src/canonical_executor.rs    CanonicalAgentLoopExecutor (default impl)
  src/default_planner.rs       DefaultPlanner with nine Default* slots;
                               pub(crate) compose_default + with_* mutators

ironclaw_reborn                             (tighter — runtime integration)
  src/text_loop_driver.rs      TextOnlyModelReplyDriver (existing, unchanged)
  src/planned_driver.rs        NEW — PlannedDriver (non-generic) implements AgentLoopDriver
  src/app_loop_family.rs       NEW — build_loop_family_registry() composition root
  src/turn_runner.rs           registers PlannedDriver instances by id            (existing)
  src/driver_registry.rs       (existing)
  src/loop_exit_applier.rs     (existing)
```

Each follow-up loop family ships as one factory function in `ironclaw_agent_loop/src/families/<name>.rs`, e.g. `coding()` returning a `LoopFamily` with select strategies swapped via `DefaultPlanner::compose_default().with_*(...)` (all `pub(crate)`). The skeleton ships none of these; only `families::default()`.

**Families live in `ironclaw_agent_loop` permanently.** The strategy traits are `pub(crate)` and the strategy-composition seal is by design — there is intentionally no escape hatch (`pub(in family-factory)` visibility, feature-gated re-exports, plugin loaders) that would let a family live in an external crate. If a family needs heavyweight external deps (tree-sitter, ripgrep, ML model bindings, etc.), the heavyweight deps are pulled into `ironclaw_agent_loop` itself as feature-gated optional dependencies — not into a sibling crate that imports strategies. The cost (one crate accumulates external deps) is borne so the sealed-strategy invariant remains structurally unbreakable. Extensions plug into the loop via hooks (§9.1), never via families. This locks the "strategies are Builtin-only" claim from §9 at the build-system level.

## 4.5 Loop family resolution

`LoopFamily` is the **top-layer abstraction**. The chain from a submitted run to the canonical executor is:

```text
RunProfile          ← user-grantable, audited
  loop_family_id: LoopFamilyId
       │
       ▼ resolved via Arc<LoopFamilyRegistry>::get(...)
LoopFamily          ← Builtin-only, sealed; opaque to downstream crates
  id: LoopFamilyId
  version: ComponentIdentity
  planner: Arc<dyn AgentLoopPlanner>   ← pub(crate) accessor
       │
       ▼ pub(crate) AgentLoopPlannerInternal extension trait
nine Strategy slots ← pub(crate); never visible outside ironclaw_agent_loop
       │
       ▼
CanonicalAgentLoopExecutor::execute_family(family, host, state)
```

Properties:

- **Profiles refer to families by `LoopFamilyId`, never by strategy composition.** A profile carries `loop_family_id: LoopFamilyId` (e.g. `"default"`); resolution maps to `Arc<LoopFamily>` via the registry. Profiles cannot enumerate strategies, override individual slots, or inject custom impls.
- **`LoopFamilyRegistry` is a Guice-style singleton.** Built once at app startup by `ironclaw_reborn::app_loop_family::build_loop_family_registry()` (the only composition root that knows which families exist), shared via `Arc<LoopFamilyRegistry>`, immutable thereafter. There is no public `register()` method; production wiring calls framework-provided family factories through the Reborn composition root.
- **Strategy traits are `pub(crate)` in `ironclaw_agent_loop`.** Extensions (Trusted or Installed) cannot implement strategies. Customization lives at the hooks layer (§9.1) — middleware around host ports, not strategy slots in the executor.
- **`PlannedDriver` is non-generic.** It holds `Arc<LoopFamily> + Arc<CanonicalAgentLoopExecutor>` and adapts them to `AgentLoopDriver`. The strategy seal means tests use real families from the registry (or `LoopFamilyRegistry::with_families` under `cfg(feature = "test-support")`), not synthetic planners.

This collapses several open questions at once: the trust-class problem (strategies are Builtin by type), the combinatorial-explosion problem (profiles get a family, not nine independent knobs), the version-drift problem (`LoopFamilyId + ComponentIdentity` pins replayable state with one primitive — see §11 — used in checkpoint payload metadata).

The full registry shape lives in [`agent-loop-briefs/loop-family-registry.md`](agent-loop-briefs/loop-family-registry.md) (WS-3.5).

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

**Family-specific durable state lives in workspace, not in `LoopExecutionState`.** Mission progress (milestones reached), plan-tree branches (for planning families), scheduled-poll cursors (for routine families), and any other family-specific durable data flows through layer 4 (host-managed) — exposed to the loop by a family-specific `HostXxxContextSource` trait composed into `LoopPromptPort`, analogous to WS-15's `HostIdentityContextSource` and the existing `HostSkillContextSource`. Strategy slots in layer 2 stay small and executor-focused (`StopStrategyState`, `GateStrategyState`, `RecoveryStrategyState`, etc.). This is what lets the framework absorb family diversity without growing the `LoopExecutionState` schema per family.

## 6. The nine strategies

Each strategy is one small Rust trait with one or two methods. Default impls model pi-mono behavior. A loop family typically swaps two or three of them; the rest stay default.

**Strategy traits are `pub(crate)` inside `ironclaw_agent_loop`.** Loop families are the public surface (`Arc<LoopFamily>` flows out of `LoopFamilyRegistry`); strategy traits are an implementation detail that family factories compose. Extensions cannot implement strategies — see §9 for the trust model and §9.1 for the hooks-as-middleware extension surface.

| Strategy | Decision it owns | Returns | Default behavior |
|---|---|---|---|
| `ContextStrategy` | What prompt mode + sections + optional inline messages to request | `LoopPromptBundleRequest` | `PromptMode::TextOnly`, all standard sections, no inline message, max 16 messages |
| `CapabilityStrategy` | Which capabilities are visible this iteration | `VisibleCapabilityFilter` | All allowed; expect provider-tool encoding |
| `ModelStrategy` | Which model preference to ask the host for | `ModelPreference` | Primary route only |
| `BatchPolicyStrategy` | Sequential vs parallel for a capability batch | `BatchPolicy` | Parallel for read-only; sequential for writes |
| `GateHandlingStrategy` | On Approval/Auth/Resource gate: block/skip/abort | `GateOutcome` (mutates `gate_state`) | Always block (checkpoint + return `LoopExit::Blocked`) |
| `RecoveryStrategy` | On capability/model error: retry/skip/abort | `RecoveryOutcome` (mutates `recovery_state`) | Retry transient model errors 2× with backoff |
| `StopConditionStrategy` | Should we stop after this completed turn? | `StopOutcome` (mutates `stop_state`) | Stop on terminate-hint; no-progress detection (see §10) |
| `InputDrainStrategy` | When to drain steering / followup queues | `(drain_steering: bool, drain_followup: bool)` | Steering before each model call; followup only when otherwise stopping |
| `BudgetStrategy` | Iteration / wall-clock limits | `IterationLimit` (+ `Option<Duration>`) | 32 iterations, no wall-clock cap |

Only `Recovery`, `Stop`, and `Gate` mutate per-strategy state and therefore return outcome enums. The other six are pure policy and return their value directly.

Inline messages — the role pi's nudge mechanism plays — are produced by `ContextStrategy` returning a `LoopPromptBundleRequest` with an `inline_messages` field. There is no separate `NudgeStrategy`; nudges are loop-family-specific context shaping.

**What `ContextStrategy` does not own:** the actual file-to-prompt assembly (loading stable identity files such as `AGENTS.md`, `SOUL.md`, and `IDENTITY.md`, merging with SKILL.md content and the transcript, projecting to the model's message format) stays in the host's `LoopPromptPort` per §9. `ContextStrategy` only picks the *request shape*; the host materializes the bundle. Identity-file content reaches `LoopContextBundle.identity_messages` via the `HostIdentityContextSource` trait introduced in WS-15 ([`agent-loop-briefs/prompt-context-assembly.md`](agent-loop-briefs/prompt-context-assembly.md)) (host-side trait owned by `ironclaw_loop_support`; see §12 for the crate-ownership rule); SKILL.md content reaches `instruction_snippets` via the existing `HostSkillContextSource`; transcript reaches `messages` via the thread service. Strategies cannot influence which files are loaded — only which `PromptMode` and standard-section selectors are requested.

Profile-scoped capability access (which tools the model sees) is similarly split: `CapabilityStrategy` picks a `VisibleCapabilityFilter` over the *already-resolved* surface; the per-run surface itself is materialized from `ResolvedRunProfile.capability_surface_profile_id` by WS-9's host-side resolver ([`agent-loop-briefs/capability-host-wiring.md`](agent-loop-briefs/capability-host-wiring.md)) — `CapabilitySurfaceProfileResolver` (host-side trait owned by `ironclaw_loop_support` per §12) — and frozen for the run.

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

    // strategy slots — one per strategy that mutates state. Stop and Gate
    // each own their own slot (no shared `control_state`) so a family's
    // future growth in either dimension can't mix concerns through a shared
    // struct.
    pub context_state:    ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state:      ModelStrategyState,    // current fallback chain index (skeleton: always 0)
    pub recovery_state:   RecoveryStrategyState, // attempt counters
    pub stop_state:       StopStrategyState,     // turns-completed, terminate-hint counters
    pub gate_state:       GateStrategyState,     // gate fingerprints / per-kind counters (empty in skeleton)
}
```

`BoundedRing<T, N>` is a small fixed-capacity ring buffer with helpers:

- `push(item: T)` — drops oldest at capacity
- `most_common_count_in(window: usize) -> usize`
- `same_run_length() -> usize`

`CapabilityCallSignature` is `(CapabilityId, ArgsHash)` — a stable hash over the capability id plus canonicalized JSON args. Lets the executor cheaply detect "same call repeated" without retaining the args themselves (no raw tool input in state, per [`turns-agent-loop.md`](contracts/turns-agent-loop.md) §6).

Strategy outcome shape (example for `RecoveryStrategy`):

```rust
pub enum RecoveryOutcome {
    Retry      { recovery: RecoveryStrategyState, scope: RetryScope, alter: Option<RetryAlteration> },
    SkipResult { recovery: RecoveryStrategyState },
    Abort      { recovery: RecoveryStrategyState, failure_kind: LoopFailureKind },
}
```

`RetryScope` is `Call` or `Iteration`, so the executor does not infer retry breadth from the alteration. Backoff alterations carry a bounded `BackoffDelayMs`, and recovery summaries use `SanitizedStrategySummary` rather than a raw `String`. The strategy returns the new value of *its own slot only*. The executor builds the next whole state by swapping that slot. The compiler enforces that `RecoveryStrategy` cannot rewrite `BudgetStrategyState`.

## 8. The canonical executor tick

Pseudocode of `CanonicalAgentLoopExecutor::execute`:

```text
state = LoopExecutionState::initial(host.run_context())  // OR ::from_checkpoint on resume

loop:
  // 0. Iteration cap at TOP of loop (not bottom). Resume with state.iteration
  //    already at limit must exit immediately, not run one more body.
  if state.iteration >= planner.budget().iteration_limit(&state):
    return LoopExit::Failed { reason_kind: IterationLimit, ... }

  // 1. Cancellation observation (top of iteration) — checkpoint + Ok(LoopExit::Cancelled(...)) if fired.
  //
  //    Cancellation is observed at this site AND at 7 additional `// CANCEL`
  //    sites below — one before each awaited strategy call. WS-6 §3.5 lists
  //    all eight boundaries explicitly; the markers here are abbreviated.
  checkpoint_and_exit_if_cancelled()

  // 2. Steering drain. LoopInputPort surface is poll_inputs(after, limit) +
  //    ack_inputs(exact_tokens). Partition to user-facing kinds only, stop at
  //    the first control input, and never ack "through" a cursor.
  // CANCEL before drain.drain_steering
  if planner.drain().drain_steering(&state):
    pending = host.poll_inputs(state.input_cursor, MAX_PER_DRAIN)
    (steering_msgs, last_consumed, exact_ack_tokens) = partition_steering_kinds(pending)
    if !steering_msgs.is_empty():
      state.append_inputs(steering_msgs)
      state.input_cursor = last_consumed
      pending_input_acks.extend(exact_ack_tokens)

  // CANCEL before context.plan_context_request
  ctx_req   = planner.context().plan_context_request(&state)
  bundle    = host.build_prompt_bundle(ctx_req)
  // CANCEL before capability.filter
  surface_filter = planner.capability().filter(&state)
  surface   = host.visible_capabilities(VisibleCapabilityRequest { filter: surface_filter })
  state.surface_version = Some(surface.version)

  checkpoint(BeforeModel, &state)  // staged via host.stage_checkpoint_payload → state_ref → port.checkpoint(kind, state_ref)
  host.ack_inputs(pending_input_acks.take())  // only after cursor is durable

  // CANCEL before model.preference
  model_pref = planner.model().preference(&state)
  model_resp = loop:
    // Wrap stream_model in a recovery loop. on_model_error is consulted
    // on failure; skeleton rejects
    // RetryAlteration::AdvanceFallback until ModelRouteChain lands (§9).
    match host.stream_model(LoopModelRequest { messages: bundle.messages,
                                              surface_version: surface.version,
                                              model_preference: Some(model_pref) }):
      Ok(resp): break resp
      Err(err):
        recovery = planner.recovery().on_model_error(&state, &sanitize_model_error(&err))
        match recovery:
          Retry { recovery, scope, alter }: state.recovery_state = recovery; honor_retry(scope, alter); continue
          SkipResult { .. }: return PlannerContract { "SkipResult on model error" }
          Abort { recovery, fk }: state.recovery_state = recovery
                                   return LoopExit::Failed { reason_kind: fk, ... }

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
        Stop { stop, GracefulStop }:
          state.stop_state = stop
          checkpoint(Final, &state)
          return LoopExit::Completed { reply_message_refs: state.assistant_refs.clone(), ... }
        Stop { stop, NoProgressDetected }:
          state.stop_state = stop
          checkpoint(Final, &state)
          return LoopExit::Failed { reason_kind: NoProgressDetected, ... }
        Stop { stop, Aborted(fk) }:
          state.stop_state = stop
          return LoopExit::Failed { reason_kind: fk, ... }
        Continue { stop }:
          state.stop_state = stop
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
            // validate_for_gate_kind() rejects approval SkipAndContinue before
            // the executor honors the strategy outcome.
            // (See WS-6 §3.3 for full match.)
          Denied(reason):
            // EmptyLoopCapabilityPort returns Denied; capability policy can
            // also deny at any time. Treat as a non-recoverable failure for
            // THIS call; consult Recovery to skip-and-continue or abort batch.
          SpawnedProcess(handle):
            // Process-wait is intentionally out of skeleton scope. Current
            // contracts have no ProcessWaiting blocked kind or resume input.
            return LoopExit::Failed { UnsupportedProcessWait, ... }
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
        Continue { stop }:        state.stop_state = stop  // fall through

  state.iteration += 1   // increment for next iteration's top-of-loop budget check
```

Three properties the canonical executor must guarantee, regardless of strategy choices:

1. **Checkpoint discipline** — checkpoints land at the four boundary kinds (`BeforeModel`, `BeforeSideEffect`, `BeforeBlock`, optionally `Final`) and nowhere else. Strategies cannot trigger checkpoints.
2. **Cancellation observation** — checked between every strategy call. On cancel: checkpoint current state, return `Ok(LoopExit::Cancelled(...))` — cancellation is a successful exit, not an executor error. (`AgentLoopExecutorError::Cancelled` is reserved for the unrecoverable edge case where the executor cannot even produce a `LoopExit::Cancelled`.)
3. **Single mutation point** — `state` is rebound in exactly one place per branch. No interior mutability, no `&mut` across strategy calls.

## 9. Cross-cutting decisions (locked)

- **Checkpoint discipline is executor-owned.** Four kinds: `BeforeModel`, `BeforeSideEffect`, `BeforeBlock`, optionally `Final`. Strategies cannot trigger checkpoints; they only return state slots.
- **Cancellation observed between strategy calls.** Strategies never see the signal directly. The canonical executor consults `LoopCancellationPort` at **eight explicit awaited boundaries** per tick (top of iteration + before each of the seven subsequent strategy-call sites — the model-response branch counts once because the Reply path and CapabilityCalls path are mutually-exclusive); the list is enumerated in WS-6 §3.5. The boundary helper name is `checkpoint_and_exit_if_cancelled` across all briefs. Adding a new strategy call to the executor MUST add a matching cancel-boundary check.
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
- **Strategies are Builtin-only (sealed at the type level).** Strategy traits are `pub(crate)` in `ironclaw_agent_loop`; `AgentLoopPlanner` is `pub` but uses the sealed-trait pattern (only types in `ironclaw_agent_loop` can implement). Extensions plug into the loop via **hooks**, which fire as middleware around host port impls composed in `ironclaw_loop_support`. Strategies decide loop-control policy; hooks intercept side-effecting port calls. They communicate only via existing `Loop*Port` DTOs and never see each other directly. The full design is in §9.1 below.
- **Loop families are bound through `LoopFamilyRegistry`,** constructed once at app startup by `ironclaw_reborn::app_loop_family::build_loop_family_registry()`. WS-3.5 lands the registry and composition root; TurnRunner selection/plumbing lands with the planned-driver/run-profile workstreams. There is no public `register()` method — the registry's contents are fixed at the composition root's compile time. See [`agent-loop-briefs/loop-family-registry.md`](agent-loop-briefs/loop-family-registry.md) (WS-3.5).
- **`LoopExit` validation is structurally enforced at the framework→reborn→turns boundary.** `AgentLoopDriver::run` / `resume` returns a raw `LoopExit`. Only `LoopExitApplier::validate(exit, LoopExitValidationPolicy)` — sealed per PR #3460, the policy type cannot be constructed by untrusted code — produces the `LoopExitValidationDecision` that flows into `TurnRunTransitionPort::apply_validated_loop_exit` via `ApplyValidatedLoopExitRequest`. The runner's transition port accepts the validated request, not a raw `LoopExit`. There is no path for an unvalidated exit to reach durable state. This mirrors the #3460 seal pattern for policies, applied at the agent-loop-framework boundary.
- **`ComponentIdentity` is the one identity primitive across the system.** `ComponentIdentity { id: Cow<'static, str>, digest: ComponentDigest }` (defined in `ironclaw_agent_loop::family` per WS-3.5) is used consistently across loop families (this PR), checkpoint payload metadata (WS-0), hooks (#3524 future), skill snapshots (#3470 future), and model routes (#3462 future). **Content-addressed only — monotonic counters are insufficient** (they false-drift when bumped without changes and false-agree when changes ship without a bump; both are silent replay-correctness bugs). The existing `LoopModelRouteSnapshot.auth_version: String` and `config_version: String` at `crates/ironclaw_turns/src/run_profile/host.rs:362` are String identities, not content hashes; they migrate to `ComponentIdentity` alongside the model-route work in #3462 — **not in this PR**. Per [`.claude/rules/types.md`](../../.claude/rules/types.md), identity-shaped values use newtypes; `ComponentIdentity` is the canonical one for component-versioning. See [`agent-loop-briefs/loop-family-registry.md`](agent-loop-briefs/loop-family-registry.md) §3.2's "Migration / propagation" subsection for the per-component migration paths.
- **JSON canonicalization for hashing follows JCS RFC 8785.** Any digest-over-JSON content in the framework — `CapabilityCallSignature::ArgsHash` is the primary case — uses [JCS RFC 8785](https://datatracker.ietf.org/doc/html/rfc8785) canonicalization. Implementation reference: the `jcs` crate (added to `ironclaw_agent_loop`'s dependencies when WS-0 ships code). Rules: object keys sorted by UTF-16 code-unit order, NaN/Infinity rejected (not valid JSON), number representation preserved (no `1.0 → 1` normalization), minimal whitespace. **Cross-model compatibility:** for typical tool-call args (strings, integers, nested objects without floats), JCS output is byte-identical to the Hermes/Forge sorted-keys-minimal-whitespace convention used in the open-weights tool-calling ecosystem (Llama 3.1+, Qwen, DeepSeek, Mistral via `<tool_call>` ChatML). Replay across model swaps (Claude ↔ Hermes 3 ↔ Llama-tool-call format) hashes identically for typical args; divergence is limited to the float-representation edge case which production tool args essentially never hit. See [`agent-loop-briefs/state-and-checkpoints.md`](agent-loop-briefs/state-and-checkpoints.md) §3.4a for the canonicalization rules in implementation form.
- **Denial telemetry surfaces through `LoopProgressPort` milestones, not the action log.** Profile-surface denials and hook denials never reach `CapabilityHost`, so they do not produce `ActionRecord` entries. Until WS-12 (`LoopProgressPort` wiring) lands, denials accumulate only in the in-memory `state.recent_failure_kinds` ring as `LoopFailureKind::PolicyDenied`. **WS-12 introduces `CapabilityBatchCompleted { denied_count: u32 }`** ([`agent-loop-briefs/loop-progress-port.md`](agent-loop-briefs/loop-progress-port.md)) which gives durable redacted denial-count telemetry without crossing into action-log territory. Per-call denial evidence (beyond counts) is out of scope for the skeleton — add a dedicated `ProfileDenialObserved` variant in a follow-up only when a real consumer demands it.

### 9.1 Strategies vs hooks: the extension contract

Strategies and hooks are two distinct extension surfaces with non-overlapping responsibilities. The agent-loop crate (`ironclaw_agent_loop`) owns strategies; the hooks crate (`ironclaw_hooks`, future per issue #3523 / #3524) owns hooks; they communicate only via the existing `Loop*Port` DTOs and never see each other directly. `ironclaw_agent_loop` has no dependency on `ironclaw_hooks` and never will.

|   | Strategies | Hooks |
|---|---|---|
| **Job** | Decide loop-control policy: what to ask the model, what to filter, when to stop, how to recover | Gate / mutate / observe / react to port calls the executor makes |
| **Crate** | `ironclaw_agent_loop` (Builtin-only, sealed) | `ironclaw_hooks` (Builtin / Trusted / Installed tiers) |
| **Where it sits** | Inside the executor — composed via the planner facade's nine slots | Around host ports — composed via `ironclaw_loop_support` host factory as middleware |
| **Sees** | `&LoopExecutionState` (refs only), strategy slots, `TurnSummary` | `LoopXxxPortRequest` / `LoopXxxPortResponse` DTOs (already redacted) |
| **Returns** | `StopOutcome`, `RecoveryOutcome`, `VisibleCapabilityFilter`, `LoopPromptBundleRequest`, … | `HookDecision::{Allow, Deny, Mutate, Pause}` mapped to existing port outcomes |
| **Mutates** | Its own state slot (immutable swap into next-tick state) | Nothing in loop state — only the request/response in flight |
| **Failure mode** | Strategy bug ⇒ executor exits with `LoopExit::Failed` | Gate/mutator fails closed; observer/effect fails isolated |

Strategies are **not** hooks. They are swappable policy slots inside the planner facade — they cannot intercept port calls, mutate prompts, or deny capabilities. They only decide what the executor asks for next. That separation is what keeps the strategy surface small (~9 traits) and what makes the hook middleware viable as a separate, additive layer.

#### Layer cake

```text
┌─────────────────────────────────────────────────────────────────┐
│  CanonicalAgentLoopExecutor          (ironclaw_agent_loop)      │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Planner facade = 9 strategy slots                       │   │
│  │   Context  Capability  Model  Batch  Gate                │   │
│  │   Recovery Stop        Drain  Budget                     │   │
│  └──────────────────────────────────────────────────────────┘   │
│                       │ calls via                               │
│                       ▼                                         │
│  AgentLoopDriverHost trait surface  (ironclaw_turns)            │
│   host.build_prompt_bundle()  host.stream_model()               │
│   host.invoke_capability()    host.finalize_assistant_message() │
└─────────────────────────────────────────────────────────────────┘
                        │
                        ▼  (middleware chain — composed in loop_support)
┌─────────────────────────────────────────────────────────────────┐
│  HookedCapabilityPort  ──►  HookedPromptPort  ──►  …            │
│   ▲ runs gate / mutator / observer hooks per port      HOOKS    │
└─────────────────────────────────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────────┐
│  Real Loop*Port impls  (HostRuntimeLoopCapabilityPort,          │
│  HostIdentityContextSource, real LlmProvider, …)                │
└─────────────────────────────────────────────────────────────────┘
```

Two clean boundaries:

1. **The executor only knows the trait** `AgentLoopDriverHost` — middleware can wrap freely without the executor noticing.
2. **Hooks wrap the trait impl, never the executor** — the strategy seal stays intact regardless of how rich the hooks layer grows.

#### Four worked scenarios

**A — Hook denies a capability the model asked for.**

```text
Model returns CapabilityCalls[shell("rm -rf /")]
Executor: invoke_capability(call)
  HookedCapabilityPort: BEFORE_CAPABILITY hooks run
    safety hook ──► Deny("destructive root path")
  Returns CapabilityOutcome::Denied { reason }
Executor: matches Denied arm
  push LoopFailureKind::PolicyDenied  ──►  RecoveryStrategy.on_capability_error()
  RecoveryOutcome::SkipResult (default)
```

The strategy layer never knew a hook was involved — it only saw `Denied`. The existing `Denied` arm handles them.

**B — Hook mutates context (additive snippet).**

```text
ContextStrategy returns LoopPromptBundleRequest{ sections: [...] }
Executor: host.build_prompt_bundle(req)
  HookedPromptPort: BEFORE_PROMPT mutator hook runs
    context_envelope hook ──► append LoopContextSnippet {
      trust = Untrusted, body = "..."
    }
  Real port assembles bundle including the patch
Executor: host.stream_model(bundle)
  Model sees the snippet, but trust-labeled and bounded.
```

Strategy never authored the snippet; hook never saw the strategy. Patches stay additive and trust-labeled per the hook design's Phase 3 contract.

**C — Hook pauses (gate).**

```text
Hook returns Pause::ApprovalRequired { refs }
HookedCapabilityPort maps to CapabilityOutcome::ApprovalRequired { ... }
Executor: matches Approval arm ──► GateHandlingStrategy
  GateOutcome::Block ──► checkpoint(BeforeBlock) ──► LoopExit::Blocked
Resume later: same machinery as a real authority gate. No new pause path.
```

Hook-induced pause is **indistinguishable from authority-induced pause** from the executor's perspective. Both produce `ApprovalRequired`, both flow through `GateHandlingStrategy`, both checkpoint at `BeforeBlock`, both resume via the existing turn input flow.

**D — Event-triggered hook (post-fact).**

```text
Loop already finalized turn ──► durable event written
ironclaw_hooks event dispatcher consumes event cursor
Effect hook enqueues a follow-up routine via normal capability dispatch
Loop is unaware. No retroactive denial possible.
```

Post-fact hooks are fully outside the executor — no skeleton change required for the hook substrate to plug in later.

#### What the skeleton already provides for hooks

The skeleton is forward-compatible with the hooks design today because:

- Executor only calls `AgentLoopDriverHost` traits — middleware can wrap freely.
- The `Denied` arm exists on the capability outcome match — without it,
  hook-deny would hit an unreachable arm. `SpawnedProcess` is recognized
  only to fail closed until an explicit process-wait contract exists.
- Gate outcomes already route through `GateHandlingStrategy` — pause-from-hook reuses pause-from-authority machinery.
- `RecoveryStrategy::on_capability_error` is the natural funnel for `Denied` reasons — hook denials feed into the same retry-budget / abort logic as any other denial.
- Checkpoint discipline is executor-owned — hooks cannot trigger checkpoints, which is the right invariant per the "hooks cannot grant authority" rule.
- Loop state is value-immutable — hooks can't mutate executor state even by accident, because they don't see `LoopExecutionState` at all.

The only thing the skeleton **doesn't** do is wrap the host ports with middleware. That's exactly the seam the future hooks PR (`ironclaw_hooks` crate per #3524) defines.

## 10. Production-safe escape

The `Default*` strategies provide three independent stuck-loop safety nets, layered:

1. **Iteration cap.** `DefaultBudgetStrategy.iteration_limit(&state)` returns `32`. Hard ceiling. Returns `LoopExit::Failed { reason_kind: IterationLimit }`.
2. **Per-error retry budget.** `DefaultRecoveryStrategy` aborts after 2 retries on a single error class. Returns `LoopExit::Failed { reason_kind: <originating-class> }`.
3. **Repetition / no-progress escape.** `DefaultStopConditionStrategy` returns `Stop { kind: NoProgressDetected }` if either:
   - the same `CapabilityCallSignature` is observed in ≥3 of the last 5 iterations, OR
   - the same `LoopFailureKind` appears ≥3 times in a row.

The "iterations" count is critical: a single iteration containing three identical calls in one batch counts as **one** observation, not three. The executor enforces this by deduplicating signature pushes within each iteration (see WS-0 §3.4 "per-iteration push semantics"). Retries of the same call within an iteration also do not re-push. This prevents a single fan-out batch from spuriously tripping the detector while still catching genuine cross-iteration loops.

The `LoopFailureKind::NoProgressDetected` variant is added in `ironclaw_turns::loop_exit` under WS-0. So is `LoopFailureKind::PolicyDenied` — emitted when a `CapabilityOutcome::Denied` (including hook-induced denials from the middleware composition seam) reaches the recovery path with no further retry. Distinct from `CapabilityProtocolError` so the no-progress detector counts repeated denials without conflating them with transport faults.

**All three safety nets are enforced by Builtin code** (iteration cap by the executor itself; retry budget by `DefaultRecoveryStrategy`; no-progress by `DefaultStopConditionStrategy`). Because strategies are sealed (§9), the retry budget and no-progress detection sit in audited code that extensions cannot replace. The iteration cap is the additional structural defense — the only one that survives even if a future audit of `DefaultRecoveryStrategy` or `DefaultStopConditionStrategy` finds a bug. Cancellation can therefore stay cooperative (observed between strategy calls) without needing preemptive `tokio::select!` boundaries at every strategy call site.

**Retry budgets are bounded within a single iteration, not across resumes.** The retry loop inside `execute_capability_batch` (master doc §8) mutates `state.recovery_state.attempts` in place between attempts. The four checkpoint kinds (`BeforeModel`, `BeforeSideEffect`, `BeforeBlock`, `Final`) sit at iteration boundaries — retries happen *between* checkpoints. A crash mid-retry resumes from the `BeforeSideEffect` checkpoint with `recovery_state.attempts = 0`; the retry budget resets. This is intentional — adding an `AfterRetryAttempt` checkpoint kind would produce checkpoint storms under retry pressure, and the **iteration cap (the structural net) still bounds total retries across resumes**: each resume costs one iteration toward `BudgetStrategy::iteration_limit`. A pathological "infinite retries across infinite resumes" loop terminates with `LoopExit::Failed { IterationLimit }` after `iteration_limit` iterations regardless. Strategies that need stronger per-call retry durability than this can model retry attempts via `recent_failure_kinds` (which IS checkpointed) at the cost of the bucketing limitation described in WS-2's recovery brief.

**Checkpoint schema migration: in-flight `Blocked` runs are NOT silently resumed against a changed digest.** When a `LoopFamily`'s `ComponentIdentity.digest` changes (any strategy composition change in the family factory, or any code change inside a strategy that affects its content hash), in-flight `Blocked` runs whose checkpoint was produced by the prior digest cannot be safely resumed — the saved state may reference internal layouts or invariants the new code no longer honors. The resume path returns `LoopExit::Failed { reason_kind: CheckpointUnavailable }`; the run is terminated and users can re-submit. The framework **never** silently resumes against the new digest — that would be the silent-failure pattern the safety nets exist to prevent. This behavior is identical to `CHECKPOINT_SCHEMA_ID` mismatch detection (per WS-10 §3.5); the `ComponentIdentity.digest` is treated as part of the schema for resume-eligibility purposes. Operators deploying a strategy change should expect a small number of in-flight blocked runs to fail with `CheckpointUnavailable` and plan accordingly (e.g., quiesce blocked runs before a rolling deploy, or accept the cost).

**Stable identity context is process-pinned in WS-15.** WS-15 caches the
stable identity candidate snapshot in the constructed context port so a
single in-process run does not re-read identity files on every prompt
build. Durable checkpoint-pinned identity snapshots are not part of
WS-15: adding snapshot refs, digest validation, and resume-time
fail-closed behavior belongs to the checkpoint/resume workstream that
owns durable run metadata.

Loop families that legitimately repeat (e.g. routines polling the same capability on schedule) opt out by swapping `StopConditionStrategy` for one that ignores the signature ring.

## 11. What this skeleton is not

The skeleton (WS-0..WS-8) ships the framework crate, traits, default strategies, executor, driver adapter, and integration tests. It deliberately does NOT ship the host-port wiring, capability execution, durable persistence, or driver registration that an end-to-end agent loop needs. Those land as the follow-up workstreams documented in §12.

- **Not a tool-capable driver runtime.** `PlannedDriver(DefaultPlanner, CanonicalExecutor)` is itself tool-capable, but it can only execute capabilities once `LoopCapabilityPort` is wired (WS-9 — see [`agent-loop-briefs/capability-host-wiring.md`](agent-loop-briefs/capability-host-wiring.md) for the full design, including profile-scoped surface filtering; adapter impl lands in `ironclaw_loop_support`, see §12). Until then, capability calls still hit `EmptyLoopCapabilityPort` and fail closed.
- **Not an identity-file context surface** (WS-15 — see [`agent-loop-briefs/prompt-context-assembly.md`](agent-loop-briefs/prompt-context-assembly.md); adapter impl lands in `ironclaw_loop_support`, see §12). `LoopContextBundle.identity_messages` stays empty until the `HostIdentityContextSource` adapter lands; `AGENTS.md`, `SOUL.md`, `USER.md`, `IDENTITY.md`, `HEARTBEAT.md`, `TOOLS.md`, `BOOTSTRAP.md`, and `context/assistant-directives.md` are not yet injected into the system prompt.
- **Not a checkpoint backing store** (WS-10). The schema id `reborn:default-loop-v1` is reserved by WS-0; the producer is the follow-up.
- **Not a `LoopInputPort` implementation** (WS-11). Steering/followup queues stay non-functional.
- **Not a `LoopProgressPort` implementation** (WS-12). Executor milestone emission is no-op until wired.
- **Not a cancellation accessor on the host** (WS-13). The executor's cancellation-observation point in WS-6 §3.5 is documented but the host method it calls doesn't exist yet.
- **Not driver registration or run-profile selection** (WS-14). Submitted turns still resolve to the existing `TextOnlyModelReplyDriver` until the registry + resolver land.
- **Not a migration of `TextOnlyModelReplyDriver`.** Existing driver stays as-is until tool-capable driver work makes the migration worthwhile.
- **Not a `prepareNextTurn`-style mid-run model swap** beyond the (deferred) fallback chain mechanism (`ModelRouteChain`).
- **Not a `MessageProjectionStrategy`.** Host owns projection.
- **Not a `NudgeStrategy`.** Inline messages flow through `ContextStrategy`.
- **Not loop-family factories beyond `default_family`.** Skeleton ships `families::default()` resolved through `LoopFamilyRegistry`. Hypothetical `routine`, `mission`, `coding`, `planning` families are stress-tested for trait-shape fit in §12.5 but ship only when there's a concrete consumer.
- **`LoopFamily` IS the top-layer abstraction.** It carries `id: LoopFamilyId`, `version: ComponentIdentity`, and an opaque planner (`Arc<dyn AgentLoopPlanner>`, sealed). `LoopFamilyId + ComponentIdentity` replaces the previously-proposed `PlannerId` newtype in checkpoint payload metadata; the same primitive is reused across replay validation, profile resolution, and future hook/skill snapshot identities (one `ComponentIdentity` shape, not four). Richer driver-side descriptors still live on `AgentLoopDriverDescriptor`, but the framework's stable identity is `LoopFamily`.

## 12. Follow-up workstreams for end-to-end execution

These workstreams convert the skeleton from "framework that compiles and tests against mocks" to "agent loop that actually runs end-to-end against the host runtime." Each one is independently scopable; together they close every gap in §11 between the skeleton and a working tool-using loop.

| ID | Title | Crates | Brief | Unblocks |
|----|-------|--------|-------|----------|
| WS-9 | LoopCapabilityPort wired to host runtime, with profile-scoped surface | `ironclaw_turns` + `ironclaw_loop_support` + `ironclaw_reborn` | [`capability-host-wiring.md`](agent-loop-briefs/capability-host-wiring.md) | Replace `EmptyLoopCapabilityPort` with `HostRuntimeLoopCapabilityPort` wrapping `CapabilityHost` (auth + approvals + audit at action time). Add `VisibleCapabilityRequest.filter` as the strategy-filter wire path. Add `CapabilitySurfaceProfileFilter` decorator that materializes a `CapabilityAllowSet` snapshot from `ResolvedRunProfile.capability_surface_profile_id` via a host-owned `CapabilitySurfaceProfileResolver`, frozen per run (§5 layer-1). Capability calls actually execute; the model sees only the strategy-narrowed profile surface. |
| WS-10 | Checkpoint store + resume path | `ironclaw_turns` (trait extension) + `ironclaw_reborn` | [`checkpoint-store-and-resume.md`](agent-loop-briefs/checkpoint-store-and-resume.md) | Add `load_checkpoint_payload(LoadCheckpointPayloadRequest) -> LoadedCheckpointPayload` to `LoopCheckpointPort`. Extend the existing `HostManagedLoopCheckpointPort` (in `ironclaw_reborn`) with the read path over its already-composed `CheckpointStateStore` + `LoopCheckpointStore`; no new adapter. Wire `PlannedDriver::resume` against the load path. Resume from `Blocked` actually works. |
| WS-11 | LoopInputPort implementation | `ironclaw_turns` + `ironclaw_loop_support` (+ host-runtime queue PR) | [`loop-input-port.md`](agent-loop-briefs/loop-input-port.md) | Define a neutral `HostInputQueue` trait and `HostQueueLoopInputPort` adapter implementing `poll_inputs` + exact-token `ack_inputs`. Steering messages reach the model mid-loop; followup messages restart the loop after a natural stop; control inputs cannot be consumed by cursor-through acks. Concrete queue substrate is a host-runtime PR. |
| WS-12 | LoopProgressPort wiring | `ironclaw_turns` (additive variants) + `ironclaw_reborn` | [`loop-progress-port.md`](agent-loop-briefs/loop-progress-port.md) | Adds additive `LoopProgressEvent` variants (`IterationStarted`, `PromptBundleBuilt`, `CapabilityBatchStarted/Completed`, `GateBlocked`, `CheckpointWritten`) and matching `LoopHostMilestoneEmitter` methods. Extends the existing `HostManagedLoopProgressPort` match (in `loop_driver_host.rs`) and keeps the durable runtime-event adapter explicit about which progress milestones are intentionally not collapsed into lossy `RuntimeEventKind` rows. |
| WS-13 | Cancellation accessor on AgentLoopDriverHost | `ironclaw_turns` (trait extension) + `ironclaw_loop_support` | [`host-cancellation-accessor.md`](agent-loop-briefs/host-cancellation-accessor.md) | Add `LoopCancellationPort` (one sync method, idempotent reads) to the `AgentLoopDriverHost` supertrait list; ship `RunStateLoopCancellationPort` backed by a host-runtime-owned `RunCancellationHandle`. Locks the WS-6 §3.5 placeholder. |
| WS-14 | PlannedDriver registration + run-profile selection | `ironclaw_reborn` + `ironclaw_turns` | [`planned-driver-registration.md`](agent-loop-briefs/planned-driver-registration.md) | Register `default_planned_driver()` in `DriverRegistry` under `reborn:planned-default` v1 with checkpoint schema `reborn:default-loop-v1` v1; add the `reborn-planned-default` profile to the resolver; add the resolver hook that can make the planned profile the implicit no-profile default. This is registration/profile selection only; live runtime wiring moves to WS-16 and product cutover moves to WS-17. |
| WS-15 | Prompt context assembly: identity-file surface | `ironclaw_turns` + `ironclaw_loop_support` + `ironclaw_reborn` + `src/workspace` | [`prompt-context-assembly.md`](agent-loop-briefs/prompt-context-assembly.md) | Add `HostIdentityContextSource` trait (analogous to existing `HostSkillContextSource`) and the concrete `WorkspaceIdentityContextSource`. Wire it through `ThreadBackedLoopContextPort::load_loop_context()` so `LoopContextBundle.identity_messages` is populated from stable workspace identity files (`SOUL.md`, `AGENTS.md`, `IDENTITY.md`, `TOOLS.md`, `BOOTSTRAP.md`) using primary-scope reads. `USER.md` and `context/assistant-directives.md` stay excluded until explicit run-context privacy policy exists. Filename canon is shared with `ironclaw_memory::safety::DEFAULT_PROMPT_PROTECTED_PATHS`; `HEARTBEAT.md` stays excluded from the stable identity prefix until a run-kind/heartbeat signal exists. |
| WS-16 | Reborn runtime wiring + real default-path smoke | `ironclaw_reborn` + `ironclaw_reborn_cli` + `ironclaw_loop_support` + `ironclaw_turns` | [`live-runtime-wiring.md`](agent-loop-briefs/live-runtime-wiring.md) | Compose the real Reborn runtime default path: registry with text-only + planned drivers, resolver with planned implicit default, coordinator, runner, and `RebornLoopDriverHostFactory` backed by the WS-9/10/11/12/13/15 adapters. Fix planned host creation so planned runs use profiled capabilities instead of the empty text-only host path. First true Reborn runtime default-path smoke; still not product live cutover. |
| WS-17 | Product-live readiness evidence | `ironclaw_host_runtime` + product turn entrypoint crates + `ironclaw_product_workflow` tests | [`product-live-cutover.md`](agent-loop-briefs/product-live-cutover.md) | Prove a product-facing no-profile turn can exercise the WS-16 composition, add fail-closed readiness checks for required adapters/config, preserve explicit text-only rollback/profile routing, and document remaining production cutover limits. This does not by itself make the production app/gateway default path live. |

**Minimum live-default path:** WS-9 + WS-10 + WS-11 + WS-12 + WS-13 + WS-15 + WS-14 are the smallest combination that completes the library/framework prerequisites for a planned loop. That set is still not live. WS-16 is the first Reborn runtime default-path smoke because it composes the registry, resolver, coordinator, runner, host factory, and real adapters with no `MockAgentLoopDriverHost`. WS-17 adds the product-facing readiness gate and local product workflow evidence. The production app/gateway default path is still blocked on the external cutover issue, and tool-result truthfulness remains blocked until product-visible tool-result evidence lands.

**Sequencing:** WS-9, WS-10, WS-11, WS-12, WS-13, and WS-15 run in parallel after the skeleton (WS-0..WS-8) lands; none of those six depend on each other. WS-14 lands after the adapter set has enough shape to make the planned profile meaningful, but it remains a registration/profile workstream. WS-16 has a single parent: the integrated WS-14 result, which includes the WS-9/10/11/12/13/15 adapter prerequisites. WS-17 depends only on WS-16. All follow-up briefs live under [`agent-loop-briefs/`](agent-loop-briefs/) — pre-written rather than scoped-at-pickup — because each answers a concrete external question (profile-scoped tool surface, identity-file to prompt wiring, durable checkpoint shape, queue substrate, milestone surface additions, sync-snapshot cancellation, driver registration, runtime composition, product cutover) whose answers are stable enough to commit before implementation begins.

**Crate ownership rule for follow-ups:**
- **Trait extensions** (new methods on `LoopXxxPort`, new variants on `LoopFailureKind`, etc.) live in `ironclaw_turns`. The contracts crate is the single source of truth for runner-facing API shape. Per `crates/ironclaw_turns/CLAUDE.md`, this crate must not depend on `CapabilityHost`, dispatcher, or runtime-lane adapters — concrete tool/identity types stay below.
- **Host-runtime adapters** (concrete `LoopXxxPort` impls that consult host backends) live in `ironclaw_loop_support`. Today this houses `ThreadBackedLoopContextPort`, `ThreadBackedLoopTranscriptPort`, `ThreadBackedLoopModelPort`, `EmptyLoopCapabilityPort`. WS-9, WS-11, WS-12, WS-15 land their impls here. WS-9 specifically adds `HostRuntimeLoopCapabilityPort`, `CapabilitySurfaceProfileFilter`, and the `CapabilitySurfaceProfileResolver` trait; WS-15 adds `HostIdentityContextSource`, while the concrete workspace reader lives in `src/workspace`.
- **Driver-side integration** (registry wiring, `LoopExitApplier`, `PlannedDriver` registration, run-profile resolution) lives in `ironclaw_reborn`. WS-13 (cancellation) splits: trait method to `turns`, accessor wiring to `loop_support`. WS-14 registers the driver/profile in `reborn` and adds the implicit-default selector in `ironclaw_turns`. WS-15 plumbs the optional identity source into `TextOnlyModelReplyDriverConfig` and `PlannedDriverConfig` from this layer. WS-16 consumes those helpers to build the real Reborn runtime composition. WS-17 owns product-live readiness checks and scoped product workflow evidence; production product binding/run ownership and gateway cutover remain outside the loop code.

This rule disambiguates the "loop_support + reborn" hedges in the table above: each row's primary owner is `loop_support` for the impl, with the `reborn` portion limited to wiring the impl into `AgentLoopDriverHost` composition / registry registration.

**Deferred-not-required:** `ModelRouteChain` (master doc §9), loop-family factories beyond `families::default()`, and migration of `TextOnlyModelReplyDriver` are useful but not on the E2E critical path. Ship them when there's a concrete consumer.

## 12.5 Loop families: anticipated and their strategy overrides

The framework is intentionally broad-scope — it serves multiple loop families through strategy variation, not just text-tool-use. To validate that the nine-strategy axis can actually express anticipated family diversity (without requiring a different executor per family), the table below enumerates the families we anticipate and the strategies each would override.

**Only `default_family` ships in the skeleton.** Every other row is hypothetical, included to stress-test the trait shapes against family diversity before WS-1/2/3 seal them. If any anticipated family would require a strategy slot that the current trait shape can't express, that's a gap to fix in this skeleton — not in a downstream PR.

| Family | Status | Strategies it would override | Where durable family-specific state lives | Stress-test result |
|---|---|---|---|---|
| `default_family` | **Ships in skeleton** | (none — uses every `Default*Strategy`) | n/a (text-tool-use baseline carries no durable family state) | ✓ baseline |
| `routine_family` | Hypothetical | `StopConditionStrategy` → `IgnoreRepetitionStop` (routines re-issue the same calls intentionally); `CapabilityStrategy` → fixed pre-curated `VisibleCapabilityFilter`; `InputDrainStrategy` → poll-on-schedule (drain when `state.iteration % interval == 0`) | Scheduled-poll cursors live in workspace; surfaced through a hypothetical `HostRoutineContextSource` (analogous to WS-15) | ✓ trait shapes accommodate. Indefinite runtime is a scheduler concern (re-invoke), not a loop concern (`BudgetStrategy::iteration_limit: u32` is fine; each routine invocation is finite) |
| `mission_family` | Hypothetical | `GateHandlingStrategy` → aggressive `Block` on every approval gate; `BudgetStrategy::wall_clock_limit` → `None`; `ContextStrategy` → load mission plan + completed milestones | Mission progress / milestones in workspace; surfaced through a hypothetical `HostMissionContextSource` | ✓ trait shapes accommodate. Mission state flows through prompt content, not strategy slots; `LoopExit::Blocked` + checkpoint resume handles multi-day pauses natively |
| `coding_family` | Hypothetical | `CapabilityStrategy` → filter to coding tools; `BudgetStrategy::iteration_limit` → higher (e.g. 100); `BatchPolicyStrategy` → sequential (file edits are causally dependent) | n/a — model carries plan state through transcript; no separate durable state | ✓ trait shapes accommodate. Coding is mostly a capability-surface choice, not a loop-strategy choice |
| `planning_family` | Hypothetical | `BatchPolicyStrategy` → sequential always; `BudgetStrategy::iteration_limit` → high; `ContextStrategy` → load "branches explored so far" | Plan tree / search state in workspace; surfaced through a hypothetical `HostPlanTreeContextSource` | ✓ trait shapes accommodate. "Backtracking" is model-driven (re-issue tool calls for an earlier branch); framework does not need a special backtrack primitive |

Two specific design notes that fall out of this exercise:

1. **`ControlStrategyState` is split into `StopStrategyState` + `GateStrategyState`** in WS-0, so future families can grow either independently without mixing concerns through a shared struct (e.g. a routine family that grows `StopStrategyState` with scheduled-poll bookkeeping does not perturb gate-handler invariants used by mission families).
2. **`HostXxxContextSource` is the universal pattern for family-specific durable context** (per §5). Each anticipated family above gets its own `HostXxxContextSource` trait that the host composes into `LoopPromptPort`; the strategy doesn't carry the state, the prompt does. This is the same shape WS-15 uses for identity files and the existing `HostSkillContextSource` uses for skills.

If a future family genuinely requires a strategy shape outside this enumeration, the skeleton's trait shapes (especially `RecoveryOutcome` and `StopOutcome`) need to be revisited before the family lands — not as a downstream amendment.

Briefs for these follow-ups will land under [`agent-loop-briefs/`](agent-loop-briefs/) with filenames matching their workstream titles. They are intentionally not pre-written here — each should be scoped against the actual code state at the time it's picked up, not the skeleton's snapshot.

## 13. Workstream map

Ten implementation briefs live in [`agent-loop-briefs/`](agent-loop-briefs/). Briefs run in parallel within a layer; dependency edges shown.

| ID | Brief | Crate(s) | Depends on |
|----|-------|----------|------------|
| WS-0 | [`state-and-checkpoints.md`](agent-loop-briefs/state-and-checkpoints.md) — `LoopExecutionState`, slots (`StopStrategyState`, `GateStrategyState`, …), `BoundedRing`, `CapabilityCallSignature`, checkpoint payload schema, `LoopFailureKind::NoProgressDetected` + `::PolicyDenied` | `ironclaw_agent_loop` + `ironclaw_turns` | — |
| WS-1 | [`strategy-traits-alpha.md`](agent-loop-briefs/strategy-traits-alpha.md) — `ContextStrategy`, `CapabilityStrategy`, `ModelStrategy` (all `pub(crate)`) | `ironclaw_agent_loop` | WS-0 |
| WS-2 | [`strategy-traits-beta.md`](agent-loop-briefs/strategy-traits-beta.md) — `BatchPolicyStrategy`, `GateHandlingStrategy`, `RecoveryStrategy` (all `pub(crate)`) | `ironclaw_agent_loop` | WS-0 |
| WS-3 | [`strategy-traits-gamma.md`](agent-loop-briefs/strategy-traits-gamma.md) — `StopConditionStrategy`, `InputDrainStrategy`, `BudgetStrategy` (all `pub(crate)`) | `ironclaw_agent_loop` | WS-0 |
| WS-3.5 | [`loop-family-registry.md`](agent-loop-briefs/loop-family-registry.md) — `LoopFamilyId`, `ComponentIdentity`, `LoopFamily`, `LoopFamilyRegistry`, `families::default()`, composition root in `ironclaw_reborn` | `ironclaw_agent_loop` + `ironclaw_reborn` | WS-0 |
| WS-4 | [`planner-facade.md`](agent-loop-briefs/planner-facade.md) — `AgentLoopPlanner` (sealed `pub` trait) + `AgentLoopPlannerInternal` (`pub(crate)`) + `DefaultPlanner` with `pub(crate) compose_default` | `ironclaw_agent_loop` | WS-1, WS-2, WS-3 |
| WS-5 | [`default-strategies.md`](agent-loop-briefs/default-strategies.md) — nine `Default*Strategy` impls | `ironclaw_agent_loop` | WS-1, WS-2, WS-3 |
| WS-6 | [`canonical-executor.md`](agent-loop-briefs/canonical-executor.md) — `AgentLoopExecutor` + `CanonicalAgentLoopExecutor::execute_family(&LoopFamily, …)` | `ironclaw_agent_loop` | WS-4, WS-5, WS-3.5 |
| WS-7 | [`planned-driver-adapter.md`](agent-loop-briefs/planned-driver-adapter.md) — non-generic `PlannedDriver` adapter (holds `Arc<LoopFamily>`); `from_family` + `from_registry` constructors | `ironclaw_reborn` | WS-6 |
| WS-8 | [`e2e-integration-tests.md`](agent-loop-briefs/e2e-integration-tests.md) — feature-gated `test_support` module + cross-crate integration suite (happy paths, safety nets, strategy intersections, state lifecycle, reborn-side driver e2e) | `ironclaw_agent_loop` + `ironclaw_reborn` | WS-7 |

Realistic parallelism: WS-0 ships first; then WS-1/2/3 and WS-3.5 land in parallel; then WS-4/5 in parallel; then WS-6; then WS-7; then WS-8 closes the suite. WS-8 is the proof-of-life — `cargo test --workspace --features ironclaw_agent_loop/test-support` only goes green when every prior workstream is correctly composed.

## 14. Glossary

- **Driver** — runner-facing trait `AgentLoopDriver` (`ironclaw_turns`). Single job: the contract `TurnRunner` calls. Implementations either bake a whole loop (legacy `TextOnlyModelReplyDriver`) or adapt the framework (`PlannedDriver`, non-generic).
- **Planner** — `AgentLoopPlanner` (`pub`, sealed). Composition of nine strategies that defines a loop family. Strategy access lives on the `pub(crate)` extension trait `AgentLoopPlannerInternal`; extensions can hold `&dyn AgentLoopPlanner` but cannot reach into strategies.
- **Executor** — `AgentLoopExecutor`, the canonical tick body. Public entry point is `execute_family(&LoopFamily, host, state)`.
- **Strategy** — one swappable decision-procedure consulted by the executor at a specific point in the tick. All nine strategy traits are `pub(crate)` in `ironclaw_agent_loop`; only Builtin code can implement them. Extensions plug into the loop via hooks (§9.1), not strategies.
- **State** — `LoopExecutionState`, value-immutable, rebound per tick. Strategy slots include `StopStrategyState` and `GateStrategyState` separately (no shared `control_state`).
- **Run context** — `LoopRunContext`, immutable for the entire claimed run.
- **Loop family** — `LoopFamily` (Builtin, sealed). Carries `id: LoopFamilyId`, `version: ComponentIdentity`, and an opaque planner. Constructed only by `families::*` factories. Resolved from `LoopFamilyRegistry` by id. Skeleton ships only `families::default()`.
- **Loop family registry** — `LoopFamilyRegistry`, Guice-style singleton built once at app startup by `ironclaw_reborn::app_loop_family::build_loop_family_registry()`. No public `register()`; contents fixed at composition-root compile time.
- **Component identity** — `ComponentIdentity { id: Cow<'static, str>, digest: ComponentDigest }`, content-addressed versioning primitive. One shape used across loop family versioning, checkpoint payload metadata, and future hook / skill-snapshot / model-route component identities.

## 15. Credits

The default loop mechanics — single async function, `Reply | CapabilityCalls` parent protocol, steering/follow-up queue ergonomics — are modeled on the [pi-mono](https://github.com/badlogic/pi-mono) `packages/agent` loop. Reborn's framework absorbs pi's hooks into typed ports (`LoopPromptPort`, `LoopCapabilityPort`, `LoopInputPort`) and adds production-grade safety nets (no-progress detection, retry budgets, gate suspension, evidence-validated `LoopExit`) that pi-mono's local-developer model doesn't need.

**Review history.** Several sections of this spec — §4.5 (LoopFamily resolution), §6 (sealed strategies note), §9 / §9.1 (sealed strategies + hooks-as-middleware contract + checkpoint schema migration), §10 (denial telemetry, retry-budget durability, checkpoint-schema-on-resume), §12.5 (anticipated families) — were refined in response to review feedback on the spec's first round (PR #3544). Substantive review threads from serrrfirat (correctness seams + simplicity), zmanian (structural gaps + strategy↔hook coordination), and an Opus subagent review pass (cross-doc consistency + implementation feasibility) shaped the current shape. Worked decisions live inline in the relevant sections; this spec is meant to be read standalone without consulting PR threads.
