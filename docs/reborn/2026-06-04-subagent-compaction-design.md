# Subagent + Compaction — Unified Design

Date: 2026-06-04. Author: derived from parallel subagent investigation across loop executor, subagent lifecycle, and compaction trigger paths. Revised 2026-06-04 after code-verification pass (symbol names, line numbers, and the durability foundation re-checked against the live tree).

## At a glance

Fixes three CX gaps for end users and one structural mess for codebase maintainers.

**End users get:** working background subagents (was disabled at ingress), no model errors from oversize tool results (was reactive `ShrinkContext` retry), parent/child run nesting in the WebUI (was flat list).

**Codebase maintainers get:** one stage owns post-capability logic (was split across two), one trait owns compaction policy (was hardcoded byte table), one trait owns background result delivery (was poll-cursor leak).

**Cost:** durable subagent stores become a hard blocker for background mode (was hand-waved). Three deliverables ship as three separate issues with explicit blocking deps (was one bundled mega-PR).

### Diagram 1 — CX (end-user / developer)

```text
                  BEFORE                          AFTER

  bg subagent      DISABLED                    spawn (mode=background)
  spawn               |                              |
                      v                              v
                 reject at ingress           BackgroundPolicy gate
                 ("not implemented")             |        |
                                            allow         deny
                                                |        (depth/quota)
                                                v
                                            run async,
                                            settle into mailbox

  large tool      append result 80k         append result 80k
  result               |                              |
                       v                              v
                 next turn assemble          PostCapabilityStage:
                 80k prompt --> MODEL          policy fires -->
                 ERROR (ctx blown)             skip_model + compact
                       |                              |
                       v                              v
                 ShrinkContext retry          next turn: compact
                 (reactive, slow)             FIRST, build 12k prompt
                                              (no model error, no waste)

  UI run view     flat list                  nested tree
                  - run A                     - run A
                  - child of A                  - child of A
                  - grandchild                    - grandchild
                  - run B                     - run B
                  (no parentage)              (depth-indented, badged
                                               with bg status)
```

### Diagram 2 — Codebase clarity (where logic lives)

```text
  BEFORE (the split, as originally drafted)

  CapabilityStage  ------ owns ------>  Stage B threshold check
   (capabilities.rs:367)                (inline, mixed concern)
        |
        v
  NEW collector stage  -- owns ----->  Stage A drain
                                       (separate file)

  -> TWO owners of "post-capability" seam.
  -> CompactionPolicy hardcoded as byte-table inline.
  -> LoopBackgroundChildPort exposes poll_settled(cursor).
  -> #4474 bundles 5 deliverables, blocking each other invisibly.


  AFTER (single seam owner)

   +-----------------+      +---------------------+      +-----------+
   | CapabilityStage | -->  | PostCapabilityStage | -->  | StopStage |
   +-----------------+      +---------------------+      +-----------+
                                       |
                         +-------------+-------------+
                         |                           |
                         v                           v
                 Responsibility 1            Responsibility 2
                 (compaction, ships now)     (bg drain, stubbed)
                         |                           |
                         v                           v
                CompactionPolicy::             LoopBackgroundChildPort::
                should_force_compact            drain_settled
                    |          |                    |
                ByteCapPolicy  BudgetFraction       v
                (v1)           Policy (v2 when     mailbox-shaped;
                               #4311 lands)         cursor-poll is impl
                                                    detail behind trait

  -> ONE owner; future authors look in one place.
  -> Trait swaps absorb #4311 with no call-site change.
  -> Trait contract = "drain my mailbox," impl can evolve.
  -> #4474 split into 3 tracked issues with blocking deps.
```

### Tech-lead overview

Seam fix = highest-leverage clarity win. Original draft put compaction-threshold logic *inside* CapabilityStage's `handle_capability_outcome` while adding a new stage for background drain — two owners, one seam, contradicting the doc's own thesis. Revision collapses both into `PostCapabilityStage` (position 7→8 in the executor pipeline) with two responsibilities. Responsibility 1 (compaction check) ships day one; Responsibility 2 (drain) is a no-op stub until durable stores land. Future maintainers grep one stage name to find all post-capability logic.

Two traits guard against rework. `CompactionPolicy` lets `ByteCapPolicy` v1 ship now and `BudgetFractionPolicy` v2 drop in when #4311 lands without touching the stage. `LoopBackgroundChildPort::drain_settled` is shaped as a mailbox drain, not a cursor poll, so the trait survives wide fan-out and slow parents — implementation can still cursor-poll behind it.

Biggest CX change for end users: proactive compaction path. `force_compact_on_next_iteration` alone would have built the oversize prompt on the next turn before compacting. Adding `skip_model_this_iteration` makes PromptStage compact *before* assembling the prompt and short-circuits the model call that turn — net: no wasted 80k-token serialize, no model error.

Background mode itself ships behind a feature toggle (off until E2E green), gated by `BackgroundPolicy` per subagent flavor + `max_concurrent_background_children` quota + mailbox admission cap. Failed children inject `CapabilityOutcome::Failed`; partial fan-out (3/5 fail) continues — degraded-but-useful results, not abort.

### Example flows

1. **Large `web_fetch` returns 60 KB JSON.** Before: appended raw; next turn builds 60 KB prompt; either model errors or context budget eats whole turn. After: CapabilityStage flushes the result; `PostCapabilityStage` calls `ByteCapPolicy::should_force_compact` → true (60 KB > 32 KB cap); flags set; next turn `PromptCompactionStep` runs first, compacts to ~12 KB, builds a small prompt, model call proceeds normally. No wasted assembly.
2. **Agent fans out 5 background research subagents.** Before: rejected at ingress, fan-out impossible. After: each spawn checks `BackgroundPolicy::Allowed { max_depth }` and the per-parent `max_concurrent_background_children` cap; passes; children run async. Three settle, one fails, one still running when parent iterates. `drain_settled` returns 4 results (3 OK + 1 `Failed`); parent gets per-child outcomes, continues; the 5th is deferred to next iteration. UI nests all 5 under the parent with status badges.
3. **Process restarts between a background child terminal write and the parent's next iteration.** Before (in-memory store): result lost, parent strands forever. After (durable store + `RestartReconciler`): result replayed on boot into the mailbox; idempotency ledger keyed on `(run_id, child_run_id, terminal_kind)` makes the replay a no-op if the parent already drained it; otherwise parent picks it up on next iteration.

### Visible impact gating

The visible CX win is gated on **Step 1 only** (compaction). Steps 2–5 (durable stores, background re-enable, drain fill, UI nesting) all block on the durable-store sub-spec under #4474 — that sub-spec is the actual critical path for "users see background subagents work." Codebase clarity win is partly debt-prevention, not visible refactor — the Stage A/B split was never merged, so the gain is "future authors don't have to learn an incoherent seam" rather than "old confusion is removed."

## Problem statement

Three intertwined gaps surfaced from the past two days of triage:

1. **Background subagents are disabled.** `SpawnSubagentMode::Background` exists in the type system but `TryFrom<SpawnSubagentWireArgs>` returns `Err(background_subagents_disabled())` for any background request (rejection call-sites at `crates/ironclaw_loop_support/src/subagent_spawn_port.rs:183–188`, checking both `run_in_background: true` and `mode == Some(Background)`; the helper itself is defined at `:1223`). Users cannot fan out asynchronous work.

2. **Subagent runs are invisible to the parent UI.** `TurnRunRecord` already carries `parent_run_id`, `subagent_depth`, and `spawn_tree_root_run_id` (`crates/ironclaw_turns/src/store.rs:189–193`), but that lineage lives in the turn-store, **not in the event log**. The `RuntimeEvent` stream carries only `parent_invocation_id` (one hop) and the WebUI projection (`RunStatusProjection`) drops even that. The browser cannot render subagent runs nested under the parent.

3. **Large capability returns blow context.** Compaction is only triggered inside `PromptStage` after the next prompt is built. An 80k-token `spawn_subagent` reply is appended to state unchecked, then the next iteration assembles a bloated prompt, then compaction kicks in (or the model errors and `ShrinkContext` retries reactively). There is no proactive check at the moment a large capability result arrives.

These three gaps share one root: the loop pipeline has no stage that owns "what happens after a capability returns but before the next prompt is built." Adding that stage — plus a proactive in-stage threshold check — solves all three.

## Current pipeline (one turn)

Verified against `crates/ironclaw_agent_loop/src/executor/canonical.rs`:

```
0. CheckpointStage (cancel_if_requested)              executor.rs
0b. CheckpointStage.emit_progress (IterationStarted)  canonical.rs:56
1. BudgetStage                                        executor/budget.rs
2. InputStage (steering drain)                        executor/input.rs
3. PromptStage  ── COMPACTION DECISION HERE           executor/prompt.rs:303
4. CheckpointStage (BeforeModel)                      executor.rs
5. ModelStage                                         executor/model.rs
6. ReplyAdmissionStage + AssistantReplyStage  ──OR──  reply_admission.rs, assistant_reply.rs
7. CapabilityStage  ── spawn_subagent lands here      capabilities.rs:367 (handle_capability_outcome)
8. StopStage (observe)                                executor/stop.rs
9. InputStage (FollowUp drain, only if ReplyOnly)     executor/input.rs (canonical.rs:209)
10. StopStage (decide)                                executor/stop.rs
11. ExitStage                                         executor/loop_exit.rs
```

Step 6 is an either/or branch (assistant reply path OR capability path), not both. Step 9 (FollowUp drain) runs only when `completed_kind == TurnEndKind::ReplyOnly`.

**Compaction trigger today:** only `DefaultCompactionStrategy::should_compact` (`strategies/compaction.rs:76–80`), invoked from `PromptCompactionStep::run()` (`prompt.rs:303`). Keyed on `total_tokens >= threshold` OR `force_compact_on_next_iteration`, where `threshold = DEFAULT_CONTEXT_LIMIT_TOKENS (128_000) - reserve_tokens (20_000) = 108_000` (a derived value, not a named constant). Token snapshot updates only when the prompt bundle is built, so a fresh large capability result is not reflected until the next iteration's prompt build.

**Subagent today:** `spawn_subagent` runs inside CapabilityStage. Blocking mode emits `CapabilityOutcome::AwaitDependentRun` (`host.rs:1414`); the parent blocks at `AwaitDependentRunGateStage` (`executor/gates.rs:131`). Background mode is wired in the observer (`completion_observer.rs:180–183` has a Background branch) but the spawn port rejects the request at ingress. The Background branch calls `write_terminal_result` (which internally calls `update_parent_result_reference`) and pushes to `background_deliveries` — it skips `resume_parent`.

**Durability reality check (load-bearing):** `write_terminal_result` → `update_capability_result` writes to a `Mutex<HashMap>` in both production (`product_live_adapters.rs:281`) and local-dev (`local_dev.rs:524`). It is **RAM only — not durable.** Likewise `BoundedSubagentGateResolutionStore` (`gate_resolution.rs:38`) and `InMemoryBoundedSubagentGoalStore` (`goal_store.rs:241`) are `Mutex`+`HashMap`, lost on restart. **Any async-poll background path requires a durable result/goal/gate backend built first — this is a hard prerequisite, not a follow-up.**

## Proposed solution

**One new pipeline stage — `PostCapabilityStage` — owns the entire post-capability / pre-prompt seam.** It carries two responsibilities that ship at different times but live in one place, so there is a single owner of "what happens after a capability returns but before the next prompt is built":

- **Responsibility 1 — proactive compaction (ships first, no durability).** Active from day one. Fires on any single oversize capability result (blocking today, background later).
- **Responsibility 2 — background result drain (depends on durable stores).** A branch inside the same stage, a no-op stub until the durable result store + `LoopBackgroundChildPort` land.

The compaction check does not need its own stage type: `PostCapabilityStage`'s shell lands in build-step 1 with the drain branch stubbed, and the branch is filled in (step 4) once durability exists. *(Earlier drafts split this into "Responsibility 2" + "Responsibility 1"; review found that split two owners across CapabilityStage and a new stage, contradicting the single-seam thesis. They are now unified as Responsibilities 1 and 2 of one stage.)*

`pending_capability_bytes` is a net-new loop-state field, written by Responsibility 1 and read by Responsibility 2. The stage defines and initializes it.

**Position:** between CapabilityStage (7) and StopStage (8). Consumes `TurnCompletedStep`, returns `TurnCompletedStep` (verified type at `executor.rs:179`) — structurally compatible with the `ExecutorStage<Input>` trait (`pipeline.rs:18–26`).

### Responsibility 1 — proactive compaction (step 1, no durability)

1. CapabilityStage has already flushed the result (via `append_spawned_child_result` `capabilities.rs:673` → `append_completed_capability_result` `:691` → `push_completed_result` `capability_helpers.rs:218`, which pushes to `state.result_refs`). Compute `result_payload_bytes`.
2. Call `CompactionPolicy::should_force_compact`. The v1 `ByteCapPolicy` impl uses a per-capability byte table (e.g. `SPAWN_SUBAGENT_CAPABILITY_ID` → 48 KB, `builtin.http` → 32 KB) — those numbers are `ByteCapPolicy`'s data, not the interface. Once #4311 lands, a `BudgetFractionPolicy` impl drops in with no call-site change.
3. If the policy returns true, set BOTH `force_compact_on_next_iteration = true` (`state/slots.rs:21`; honored at `compaction.rs:76,80`) AND `skip_model_this_iteration = true` (net-new flag — see "Compaction timing" below).
4. Track per-result byte sizes in `pending_capability_bytes`.
5. Emit `LoopProgressEvent::CompactionScheduled { reason: CapabilityResultOverflow }` (closes part of #4427 trace gap and #4313 milestone schema).

### Responsibility 2 — background result drain (step 4, depends on durable stores)

1. Call `LoopBackgroundChildPort::drain_settled(parent_run_id) -> Vec<SettledChild>` (mailbox-shaped; cursor-polling may back the v1 impl) on `AgentLoopDriverHost` (`crates/ironclaw_turns/src/run_profile/host.rs:2057`, currently 11 Loop*Port traits + Send + Sync = 13 super-traits, no background-child port). No-op stub until the durable store + port land.
2. For each drained child: hydrate its `LoopResultRef`, append to `state.result_refs`, emit a `LoopProgressEvent` so the WebUI projection sees it.
3. Update `pending_capability_bytes`.
4. Apply the per-iteration drain cap (≤K results/pass; carry the rest to next iteration — see backpressure under Extensibility).

### Compaction timing (wasted-build fix)

`force_compact_on_next_iteration` alone defers compaction by a full iteration: the next turn still assembles the oversize prompt, *then* compacts. For an 80k-token payload that is an expensive wasted serialize. So Responsibility 1 also sets `skip_model_this_iteration`; PromptStage honors it by running `PromptCompactionStep` (`prompt.rs:303`) *before* building the model prompt and short-circuiting the model call that turn. Net: detect overflow → compact → build the already-shrunk prompt, with no oversize assembly in between.

### Ordering + delivery contract

Within one pass of `PostCapabilityStage`, **Responsibility 2 (drain) runs before Responsibility 1's threshold check**, so freshly drained background results are counted in the same overflow decision. Results that settle *after* this turn's compaction snapshot are **deferred to the next iteration** — never retro-injected into an already-built prompt.

Background child outcomes:
- **Settled OK** → result appended to `state.result_refs` as a normal capability result.
- **Failed** → `SettledChild` carries a failure variant; the stage injects `CapabilityOutcome::Failed` for that child (surfaced via `LoopFailureKind`, #4427).
- **Partial fan-out** (e.g. 3 of 5 children fail) → the parent **continues**; each child's status surfaces independently. A failed child does not abort the parent batch. (Abort-on-any is explicitly rejected — fan-out value is degraded-but-useful partial results.)

### Re-enable background mode

Change in `subagent_spawn_port.rs` `TryFrom<SpawnSubagentWireArgs>` (the **only** rejection call-site — rejection at ~183–188; helper defined at ~1223; there is no `finish_spawn` function):

```rust
// Before (lines ~183-188): rejects when run_in_background || mode == Background
return Err(SpawnSubagentArgsError::background_subagents_disabled());

// After: accept Background, construct SpawnSubagentArgs { mode: Background, .. }
```

Plus:
- `SpawnSubagentArgs` and the `spawn_subagent` tool schema accept `mode: "blocking" | "background"`.
- `SubagentDefinition` (`subagent_spawn_port.rs:207`, currently `subagent_kind`/`allow_nesting`/`requested_run_profile`) gains a `background_eligible: bool` flag; runtime denies background spawns for flavors that opt out. Until added, the `TryFrom` blanket gate is the only enforcement — no per-kind granularity.
- Background spawn emits `CapabilityOutcome::Completed { payload: SpawnedChildRunPayload { child_run_id, mode: Background, status: Spawned } }` instead of `AwaitDependentRun`. Parent continues; Responsibility 2 picks the result up later.

### WebUI parent-child nesting — 5-step chain, not 3 files

The source-of-truth lineage exists on `TurnRunRecord` but is **not in the event log**, so the projection layer cannot see it today. Real chain:

1. `crates/ironclaw_events/src/runtime_event.rs:80` — `RuntimeEvent` carries only `parent_invocation_id`. Add `subagent_depth` and `spawn_tree_root_invocation_id` to the event schema.
2. Runtime emission site — populate the two new fields from `TurnRunRecord` when the run-start event is emitted.
3. `crates/ironclaw_event_projections/src/lib.rs` — `apply_run_event` (`runtime_projection.rs:157`) must fold `parent_invocation_id` + the two new fields into `RunStatusProjection` (it currently folds the run's own `invocation_id` from `event.scope.invocation_id`, but does not fold `parent_invocation_id`, `subagent_depth`, or `spawn_tree_root_invocation_id`).
4. `crates/ironclaw_event_projections/src/lib.rs:264` — extend `RunStatusProjection` with `parent_run_id`, `subagent_depth`, `tree_root_run_id`.
5. `crates/ironclaw_product_adapters/src/outbound.rs:638` — extend `ProductProjectionItem::RunStatus`; `crates/ironclaw_reborn_composition/src/projection.rs:1025` (`run_status_projection_state`) will need to pass them through (does not today).

**Alternative to steps 1–2:** projection-time join against the turn-store instead of threading through the event schema. Pick one explicitly before execution — the original "3-file change" framing only covered steps 4–5 and is insufficient.

Browser-side: render runs grouped by `tree_root_run_id`, indented by `subagent_depth`. Mark Background runs with a distinct status badge (running, settled, failed). Clicking a child run focuses its thread, piggybacking on existing thread navigation primitives — no new component needed.

## Durability requirements (hard prerequisite — tracked in #4474)

These block Responsibility 2 and reliable background mode. They are **not** optional follow-ons:

- `BoundedSubagentGateResolutionStore` and `InMemoryBoundedSubagentGoalStore` move to libsql + postgres backed stores. Restart between child terminal and parent observer dispatch no longer strands either blocking or background parents.
- The capability result store (`update_capability_result`, today a `Mutex<HashMap>`) gains a durable backend so Responsibility 2's `drain_settled` can read results written after a restart.
- `RestartReconciler` replays missed background settlement events on boot.
- Idempotency ledger keyed on `(run_id, child_run_id, terminal_kind)` so post-cleanup replay is a no-op.

**Schema + parity sub-spec (write before step 2 starts).** Step 2 is unestimable as a one-liner. Each of the three stores (goal, gate, result) needs an explicit table schema (columns, indexes), a migration for any in-flight in-memory state at deploy, dual-backend parity (repo rule: every persistence feature supports **both** libsql and postgres), and defined concurrent-write semantics when two settlement events race for the same `(run_id, child_run_id)` row. Land this as a sub-spec under #4474 first.

**Rollback.** If durable stores ship but background mode stays off: new tables are safe to leave empty; the in-memory paths remain the active fallback during rollout; `RestartReconciler` against an empty store at boot is a no-op, not an error. Rolling back the migration = drop the empty tables.

## Issue map

| Issue | Role under this design |
|---|---|
| **#4474** (subagent umbrella) | Master tracker. Scope: durable goal/gate/result stores (PREREQUISITE), re-enable Background mode, Responsibility 2 + host port, WebUI nesting, parity test |
| **#4084** (deferred — background subagent results) | Reactivated. Closed as superseded by #4474 with pointer to new scope, OR kept open as the "Background mode re-enable" sub-ticket |
| **#4366** (compaction wedge) | Independent. Responsibility 1 sets `force_compact_on_next_iteration`; PromptStage compaction path must be solid for that to land cleanly |
| **#4464** (status-only stabilization metadata) | Affected. Responsibility 1 can trigger compaction mid-batch — `CompactionTask::run` must re-validate after capability flush. Add stabilization fingerprint per #4464 |
| **#4311** (budget governance collapse) | Responsibility 1 threshold is independent of budget scope today; converges once #4311 lands |
| **#4313** (milestone payload schema) | Add `CompactionInitiator::CapabilityResultOverflow` variant |
| **#4427** (LoopFailureKind trace) | Responsibility 1 emits `LoopProgressEvent::CompactionScheduled { reason }`; trace coverage extends to that event |
| **#4365** (cancel mid-await) | Responsibility 2 polls in a tokio task; cancel must abort the poll cleanly — inherits CapabilityStage cancel discipline |
| **#4368** (loop hygiene — LoopHostDependencies bundle) | New `LoopBackgroundChildPort` joins the bundle |

## Extensibility

The seam itself extends well — Responsibility 2 is the natural home for any future "reconcile async work before deciding to loop" logic (tool-result streaming, multi-child fan-in join, speculative result eviction), and `force_compact_on_next_iteration` is a clean decoupled producer/consumer handoff that any future compaction trigger can reuse. Three design choices, however, must be made up front or they force a rip-out later.

### 1. Responsibility 1 must dispatch through a `CompactionPolicy` trait, not a byte table

A hardcoded `capability_id → byte cap` lookup (48 KB spawn, 32 KB http) is a dead end: every new capability needs a table entry, there is no principled default, and **bytes ≠ tokens** (a 32 KB JSON blob and 32 KB of prose tokenize very differently). The doc already concedes #4311 replaces it with "a fraction of remaining budget" — so building the byte table first builds a throwaway.

Instead, ship the trait on day one and put the dumb impl behind it:

```rust
trait CompactionPolicy {
    fn should_force_compact(&self, result: &LoopResultRef, budget: &BudgetState) -> bool;
}
// v1: ByteCapPolicy (per-capability table)   — ships in step 1
// v2: BudgetFractionPolicy                    — drops in when #4311 lands, no call-site change
```

Same merge effort in step 1, zero rip-out when #4311 arrives. Responsibility 1 calls the trait; the table becomes one impl, not the interface.

### 2. `LoopBackgroundChildPort` should be mailbox/drain shaped, not poll-since-cursor

`poll_settled(parent, since_cursor)` is pull-based: Responsibility 2 only collects when the parent happens to iterate. An idle parent (blocked on its own gate, or just slow) leaves settled children unobserved, and every collector has to track per-parent watermarks. That trait shape does not scale to wide fan-out or deep trees.

Shape the port around draining a per-run mailbox fed by settlement events:

```rust
trait LoopBackgroundChildPort {
    fn drain_settled(&self, parent_run_id: TurnRunId) -> Vec<SettledChild>;
}
```

Polling can still be the v1 *implementation* behind `drain_settled`, but the trait contract is "give me what settled for this run," not "poll since cursor." The `RestartReconciler` in build step 2 already implies a settlement event log exists — feed the mailbox from it. Locking the contract to a cursor now means refactoring the trait when fan-out gets wide.

### 3. Coarse booleans now → wire-contract migrations later

- **`background_eligible: bool` is too coarse.** Real policy is per-flavor × per-depth × per-budget. Model it as a policy enum from the start (`BackgroundPolicy::{Denied, Allowed { max_depth: u32 }, ...}`) — cheaper than migrating a bool through the wire contract twice once depth/budget limits arrive.

### Extensibility gaps to close before background ships

- **Depth / recursion bound.** Background subagents can spawn background subagents. `subagent_depth` exists but nothing caps the tree or budgets it — fan-out × depth = unbounded run explosion. Needs a policy knob (folds into `BackgroundPolicy::Allowed { max_depth }`) before background mode is enabled, not after.
- **Drain backpressure (Responsibility 2).** N settled children injecting N results into one parent's `result_refs` in a single pass → instant context blow → force-compact → compaction thrash. #4464 re-validation handles correctness but not admission control. Add a per-iteration drain cap (inject ≤K results/pass, carry the rest to the next iteration).
- **Concurrent fan-out quota.** Depth is bounded but horizontal width is not — one agent can spawn N background children, each a full model call (cost + resource-exhaustion vector), and a slow parent grows its mailbox unboundedly. Add `max_concurrent_background_children` (folds into `BackgroundPolicy`) plus a mailbox admission cap: refuse new background spawns when the parent's pending mailbox is full (backpressure to the spawning model).

## Seam point evidence

Pipeline mapping confirms the background-results stage at position 7→8 is the cleanest seam:

- Sits after all capability writes for the current turn are flushed
- Runs before StopStage decides whether to loop again, so injected background results can influence stop conditions
- PromptStage of the next iteration naturally sees the updated context — no PromptStage rewrite required
- `force_compact_on_next_iteration` flow is already in place; PromptStage needs one new branch to honor `skip_model_this_iteration` (compact-before-build short-circuit)

Alternative seams (inside CapabilityStage, before PromptStage, on `DefaultPlanner` as a strategy, or via CheckpointStage) all introduce ownership or sequencing problems documented in the investigation artifacts.

## Hot files (combined, verified)

- `crates/ironclaw_agent_loop/src/executor/canonical.rs` — insert `PostCapabilityStage`
- `crates/ironclaw_agent_loop/src/executor/pipeline.rs` — `PostCapabilityStage` field on the pipeline; `ExecutorStage` trait
- `crates/ironclaw_agent_loop/src/executor/capabilities.rs:367` — result flush (`handle_capability_outcome`) feeding `PostCapabilityStage` Responsibility 1; `append_spawned_child_result` at `:673`
- `crates/ironclaw_agent_loop/src/executor/capability_helpers.rs:218` — `push_completed_result` updates `pending_capability_bytes`
- `crates/ironclaw_agent_loop/src/state.rs:48` — `result_refs: Vec<LoopResultRef>` (NOT `completed_results`); add `pending_capability_bytes`
- `crates/ironclaw_agent_loop/src/state/slots.rs:21` — `force_compact_on_next_iteration` (exists)
- `crates/ironclaw_agent_loop/src/strategies/compaction.rs:42,76` — `DEFAULT_CONTEXT_LIMIT_TOKENS`, `should_compact`; Responsibility 1 threshold table per `capability_id`
- `crates/ironclaw_agent_loop/src/executor/prompt.rs:303` — `PromptCompactionStep::run()` (compaction decision)
- `crates/ironclaw_loop_support/src/subagent_spawn_port.rs:183` — re-enable Background mode in `TryFrom`; `:207` `SubagentDefinition`
- `crates/ironclaw_reborn/src/subagent/completion_observer.rs:180` — Background branch (exists); `:516` `write_terminal_result`
- `crates/ironclaw_turns/src/run_profile/host.rs:2057` — add `LoopBackgroundChildPort` to `AgentLoopDriverHost`
- `crates/ironclaw_turns/src/store.rs:189` — `TurnRunRecord` parent/depth/tree fields (source of truth)
- `crates/ironclaw_events/src/runtime_event.rs:80` — `RuntimeEvent`; add depth + tree-root fields
- `crates/ironclaw_event_projections/src/lib.rs:264` — `RunStatusProjection` field extension; `runtime_projection.rs:157` `apply_run_event` fold
- `crates/ironclaw_product_adapters/src/outbound.rs:638` — `ProductProjectionItem::RunStatus` field extension
- `crates/ironclaw_reborn_composition/src/projection.rs:1025` — `run_status_projection_state` plumbing
- Durable backends: `gate_resolution.rs:38`, `goal_store.rs:241`, `product_live_adapters.rs:281`, `local_dev.rs:524`

## Test matrix

- **restart-between-spawn-and-settle** — `RestartReconciler` replays missed settlement; parent observer receives result after reboot (replay correctness).
- **duplicate settlement delivery** — idempotency ledger keyed on `(run_id, child_run_id, terminal_kind)` makes a repeated delivery a no-op.
- **Responsibility 1 compaction-on-overflow** — single oversize capability result triggers `force_compact_on_next_iteration`; PromptStage compacts on next iteration.
- **Responsibility 2 settlement injection** — settled background child result is appended to `state.result_refs`; subsequent prompt sees it.
- **E2E background fan-out happy path** — parent spawns N background subagents; all settle; all results surface in parent context without prompt overflow.

## Build order

Reordered so the durability-free win lands first and hard prerequisites precede dependents.

1. **`PostCapabilityStage` shell + Responsibility 1 (ships standalone):** new stage at seam 7→8 with the background-drain branch stubbed as a no-op; `CompactionPolicy` trait + `ByteCapPolicy` v1; `pending_capability_bytes`; `force_compact_on_next_iteration` + `skip_model_this_iteration` set on overflow; `PromptStage` short-circuit; `CompactionScheduled` event. No durability, no background mode. **Delivers proactive compaction immediately; the trait absorbs #4311 with no call-site change.**
2. **Durable stores (HARD PREREQUISITE for everything below):** write the schema+parity sub-spec first (see Durability requirements); then goal/gate/result stores → libsql + postgres; `RestartReconciler`; idempotency ledger. (#4474 core.)
3. **Background mode re-enable:** `TryFrom` accepts Background; `SubagentDefinition.background_eligible` (→ `BackgroundPolicy`); `max_concurrent_background_children`; `Completed` payload instead of `AwaitDependentRun`. Depends on step 2.
4. **Fill Responsibility 2 (drain branch):** `LoopBackgroundChildPort` + `drain_settled` (cursor-polling may back the v1 impl) + per-iteration drain cap + delivery contract (failure variant, defer-late-arrivals). Depends on steps 2 + 3.
5. **WebUI nesting:** event-schema fields (or projection-time join) → runtime emission site (populate new fields from `TurnRunRecord` when run-start event is emitted) → fold in `apply_run_event` → `RunStatusProjection` extension → product layer → browser. Depends on step 2 durable artifacts being projection-ready.
6. **Parity test** (#4431 follow-on — already opened).

**Tracked as separate issues with explicit blocking deps:** step 1 ships independently; step 4 (drain fill) is blocked on step 2; step 5 (WebUI) is orthogonal, blocked only on step 2's projection-ready artifacts. Do not bundle into one PR — the unified design is documentation, not a single merge.

Closing criteria for #4474: steps 2–6 merged; background subagent feature toggle defaults to off until E2E green; documented contract under `docs/reborn/`. Step 1 can merge independently ahead of all of them.
