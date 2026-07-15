# Agent-Loop `canonical.rs` Slop Cleanup — Design

## Implementation Outcome (2026-07-14, updated)

**Shipped: Sections 1 and 2, both landed.**

Section 1 (completion-nudge extraction) shipped as designed with one
revision: the thermo-nuclear plan review flagged the `allow_completion_nudge:
bool` on `StopInput` as a smell (2 of 3 call sites would always pass a
constant `false`). It was replaced with a dedicated
`StopStage::decide_with_completion_nudge()` wrapper — only the main path
calls it; `ResumeApproval`/`ResumeAuth`/`ResumeExternalTool`/`SkipModel`
call the plain, unchanged `decide()`. Zero diff at those three call sites.

Section 2 (latency-instrumentation extraction) initially caused a
reproducible stack overflow in
`executor::tests::policy_denied_capability_error_honors_retry_recovery`.
**Root cause found and fixed**, after it was first misdiagnosed as
unexplainable and Section 2 was briefly abandoned: it was **not** a logic
bug. `canonical.rs`'s `execute()` was one monolithic function covering all
three branches (Prepared/ResumeApproval/SkipModel). In an unoptimized
(`cargo test`) build, rustc does not tightly reuse stack slots across
mutually-exclusive match arms, so adding the `timed()`/`_timed()` call
sites (even to branches this specific test never executes) inflated
`execute()`'s single stack frame enough that, combined with
`capabilities.rs`'s own small, pre-existing, unrelated bounded recursion
running nested inside it, the combined depth crossed the default 8 MB
thread stack. Confirmed by stubbing the untaken branches with
`unreachable!()`, which fixed the crash instantly with zero semantic
changes — proving size, not logic, was the cause.

**The fix, which is also the real Section 2 implementation:** decompose
`execute()`'s three branches into their own methods
(`execute_prepared_turn`, `execute_resume_turn`, `execute_skip_model_turn`
on `DefaultExecutorPipeline`), each returning a shared `TurnStep` enum
(`Continue{state, pending_input_ack}` | `Exit(LoopExit)`) so `execute()`'s
own dispatcher becomes a single small `loop` calling one of the three and
uniformly handling the result. This is exactly the "3x duplication" cleanup
the original spec below explicitly deferred as a Non-Goal — it turned out
to be the prerequisite for shipping Section 2 safely, not an unrelated
nice-to-have. One more fix was needed on top: clippy's
`large_enum_variant` lint caught `TurnStep::Continue`'s `state:
LoopExecutionState` field at 2224 bytes inline — boxed it
(`Box<LoopExecutionState>`), matching the existing `CancelCheck`/
`BudgetStep` pattern elsewhere in this file.

Result: 400/400 crate tests green (including 3 repeated runs of the
previously-overflowing test), clippy clean with `-D warnings`,
`cargo test -p ironclaw_architecture` green. `canonical.rs` is now four
focused functions instead of one ~570-line one.

**Not attempted: Section 3 (event-streaming cleanup) and Section 4
(guardrail rule file).** Time went to the Section 1/2 landing instead. Both
remain valid, low-risk follow-ups — Section 3 in particular
already has its ordering caveat documented below (the `PromptStage` move
changes `BeforeModel` checkpoint semantics; a real test caught this — see
Section 3's note).

## Problem

`crates/ironclaw_agent_loop/src/executor/canonical.rs` (the ordered
lifecycle spine) has grown 459 → 629 lines since it was introduced, not
through legitimate stage extension but through logic and mechanics that
should have been pushed into an owning stage instead. This is the exact
anti-pattern `crates/ironclaw_agent_loop/CLAUDE.md` already warns against
("Put lifecycle mechanics in the owning executor stage instead of adding
branch logic directly to `canonical.rs`"), and it has recurred across at
least two contributors and multiple PRs. This spec fixes the concrete
instances found and adds a guardrail so it stops recurring silently.

Three categories of slop were found, all in `canonical.rs`'s `execute()`
body:

1. **Decision-logic branching** — PR #6013 added a `completion_nudge_should_fire()`
   check plus direct mutation of `stop_state.{completion_nudges_used,
   completion_nudge_pending, last_reply_trailed_off}` inline in the
   `StopStep::Stop` match arm of the main per-iteration path only (not the
   `ResumeApproval`/`SkipModel` paths, which hit the same match shape ~150
   lines further down but don't have the check — a silent inconsistency,
   not a deliberate one).
2. **Repeated cross-cutting mechanics** — PR #5487 ("Add inner latency
   spans") wrapped ~24 call sites in `canonical.rs` with a `latency::stage!`
   macro invocation, each repeating `host.run_context()` (already
   available via `ctx.host`) and a hardcoded operation-name string. Two
   more call sites hand-roll a third, slightly different manual
   `started_at()`/`operation_ok()` pattern instead of using the macro.
   This is the single largest contributor to `canonical.rs`'s line count.
3. **A stray stage-owned decision** — the repeated-call-warning
   `driver_note` progress event and its paired
   `state.stop_state.mark_repeated_call_warning_rendered()` mutation are
   emitted directly in `canonical.rs`, gated on a flag (
   `prompt.rendered_repeated_call_warning`) that `PromptStage` (prompt.rs)
   already computes and already emits its own progress events
   (`CompactionStarted`/`CompactionCompleted`/`CompactionFailed`/
   `PromptBundleBuilt`) for. A crate-wide grep confirmed 11 of 13
   `emit_progress` call sites already live correctly inside their owning
   stage file (gates.rs, prompt.rs, capabilities.rs, model.rs,
   checkpoint.rs) — this is the only misplaced one plus `IterationStarted`
   (see Non-Goals).

## Section 1 — Completion-nudge extraction (SHIPPED)

**Owner:** `StopStage::decide()` (`executor/turn_stop.rs`). Not the
`StopConditionStrategy` trait (`strategies/stop.rs`) — that trait's
methods take only `&LoopExecutionState`, with no access to `ctx.host` /
`SteeringPolicy`, so it structurally cannot make this decision.
`StopStage::decide()` already has `ctx.host` and is the single call site
all three paths (main, resume, skip-model) go through for the
Stop-vs-Continue decision.

**As shipped** (revised post-thermo-review from the bool-flag design
originally written here):

- `completion_nudge_should_fire()` moved into `loop_exit.rs`, co-located
  with `COMPLETION_NUDGE_LIMIT` and its sibling nudge functions
  (`try_final_answer_nudge`, `completion_nudge_control_message`,
  `reply_trailed_off`) — matching the existing `budget.rs` →
  `loop_exit::try_final_answer_nudge` cross-file precedent.
- New `StopStage::decide_with_completion_nudge()` in `turn_stop.rs` wraps
  the existing, **unchanged** `decide()`: on `StopStep::Stop` where the
  nudge predicate is true, it mutates `completion_nudges_used`/
  `completion_nudge_pending`/`last_reply_trailed_off` and returns the
  **existing** `StopStep::Continue` variant — no new variant needed.
- Only the main per-iteration path in `canonical.rs` calls
  `decide_with_completion_nudge`; `ResumeApproval`/`ResumeAuth`/
  `ResumeExternalTool`/`SkipModel` call plain `decide()`, unmodified.
  The main-path-only behavior is now expressed by *which method is
  called*, not by an accidental omission.
- Net effect on `canonical.rs`: the `StopStep::Stop` arm on the main path
  returns to being a plain "call `self.exit.process`, ack, return" block
  — identical shape across all three paths. `canonical.rs` shrank
  629 → 573 lines.

## Section 2 — Latency-instrumentation extraction (SHIPPED — see outcome above; required decomposing `execute()` first)

Two kinds of call site, two extraction shapes:

**Trait `.process()` call sites** (budget, input, prompt, model,
reply_admission, assistant_reply, capabilities, post_capability,
checkpoint, exit — 12+ sites): add a default method on `ExecutorStage`
(`executor/pipeline.rs`):

```rust
async fn timed(
    &self,
    operation: &'static str,
    ctx: StageContext<'_>,
    iteration: u32,
    input: Input,
) -> Result<Self::Output, AgentLoopExecutorError> {
    let started = latency::started_at();
    let result = self.process(ctx, input).await;
    latency::result(operation, ctx.host.run_context(), iteration, started, &result);
    result
}
```

Call sites collapse from the current 5-8 line `latency::stage!(...)`
block to one line, e.g. `self.budget.timed("budget", ctx,
state.iteration, input).await?`.

**Inherent (non-trait) methods** (`StopStage::observe`/`decide`,
`CheckpointStage::cancel_if_requested`/`write`/`emit_progress`,
`PendingInputAck::ack`): each gets a small `_timed` sibling defined next
to the method it wraps, in the file that already owns that stage
(`turn_stop.rs`, `checkpoint.rs`, wherever `PendingInputAck` lives), using
the same `latency::started_at`/`latency::result` primitives. The existing
`latency::stage!` macro stays as the shared low-level primitive both the
trait default and the per-stage wrappers call internally — it is not
removed, just no longer invoked directly from `canonical.rs`.

Side effect: the `_resume`/`_skip_model`-suffixed operation names (3x
repeated `stop_observe`/`stop_decide`/`exit`/`ack` blocks in the three
parallel paths) shrink from long macro blocks to one-line calls each,
meaningfully cutting `canonical.rs`'s line count without touching the
3-path structural duplication itself (see Non-Goals).

This section was designed as a pure mechanical extraction with no
behavior change — the stack-overflow regression documented in
"Implementation Outcome" above was unexpected and its root cause is
unconfirmed. **Do not re-attempt this exact implementation without first
understanding why it broke** `policy_denied_capability_error_honors_retry_recovery`.

## Section 3 — Event-streaming cleanup (NOT ATTEMPTED)

- Move the repeated-call-warning block (`state.stop_state
  .mark_repeated_call_warning_rendered()` + the `driver_note` progress
  emission) out of `canonical.rs` and into `PromptStage::process`
  (prompt.rs), right where `rendered_repeated_call_warning` is already
  computed. Deletes the block from `canonical.rs` entirely; no new types.
- **Ordering caveat found during a prior attempt at this session (then
  reverted): this is NOT purely mechanical.** `canonical.rs` currently
  writes the `BeforeModel` checkpoint *before* running this block, so the
  checkpoint captures `RepeatedCallWarningPhase::PendingRender`. Moving
  the mutation into `PromptStage::process` (which runs *before* the
  `BeforeModel` checkpoint write in `canonical.rs`) would make the
  checkpoint capture `Rendered` instead — a real behavior change, caught
  by the existing test
  `repeated_call_warning_checkpoint_stays_pending_until_model_request`.
  Any future attempt must preserve checkpoint-write ordering relative to
  this mutation, not just relocate the code.
- `IterationStarted` emission stays a `canonical.rs` call — it marks the
  top of each loop tick, which is genuinely the spine's own bookkeeping,
  not any single stage's decision.

## Section 4 — Guardrail: new rule file (NOT ATTEMPTED)

Add `.claude/rules/agent-loop-canonical-branching.md`, mirroring the
existing incident-driven style of `.claude/rules/agent-loop-capabilities.md`
(concrete repro signature, the two failure paths, a review flag). Content:

- **Title/why**: this exact class of drift has now shipped in two
  different PRs (#5487 latency spans, #6013 completion nudge) by two
  different contributors — a smoke alarm the existing `CLAUDE.md` prose
  rule didn't catch in review.
- **The rule**: any new decision logic (stop/nudge/retry/gate policy) or
  repeated cross-cutting mechanics (timing, tracing, progress emission)
  goes into the stage that already owns the related state/decision — never
  appended inline to `canonical.rs`'s `execute()` body. If no existing
  stage owns it, that's a signal a new stage is needed, not a signal to
  inline it.
- **Review flag**: a PR diff to `canonical.rs` that adds a new `if`/`match`
  arm touching `state.*` fields, or a new hand-rolled timing/tracing call,
  without a corresponding stage-file change, should be rejected or asked
  to justify why the logic doesn't belong in a stage.
- **Worked examples**: both incidents from this spec (with file:line style
  pointers), matching how `agent-loop-capabilities.md` documents its two
  incidents.
- Cross-reference from `crates/ironclaw_agent_loop/CLAUDE.md`'s existing
  "Executor stage ownership" section to the new rule file, the same way
  other rule files cross-link.

## Testing

- **Nudge (Section 1, shipped):** #6013's existing WITH/WITHOUT-gate pair
  through `CanonicalAgentLoopExecutor` remains the regression test for the
  main path (all still pass unchanged). Added
  `plain_decide_never_nudges_while_decide_with_completion_nudge_does` — a
  `StopStage`-level test (mirrors the precedent of
  `capability_stage_denied_approval_resume_surfaces_gate_declined_failure_and_continues`,
  which also tests a stage directly rather than through the full executor)
  proving `decide()` never nudges even when the stop kind is
  unconditionally nudge-eligible (`NoProgressDetected`), while
  `decide_with_completion_nudge()` does under the identical input. Full
  crate suite: 400/400 passing (399 pre-existing + 1 new), clippy clean
  with `-D warnings`, `cargo test -p ironclaw_architecture` green.
- **Latency (Section 2):** N/A — abandoned before landing.
- **Event streaming (Section 3):** N/A — not attempted.

## Non-Goals (explicitly out of scope)

- **The 3x duplicated `stop.observe` → `stop.decide` → `exit` sequence**
  across the main/`ResumeApproval`/`SkipModel` paths in `canonical.rs` is
  a separate, larger structural question (the three paths differ in what
  happens *between* observe and decide — e.g. only the main path drains
  follow-up input) and is not addressed here.
- No changes to `strategies/stop.rs`, `strategies/CLAUDE.md`, or any
  public strategy trait signature.
- No changes to `capabilities.rs`, `capability_helpers.rs`, or other large
  executor files — this spec is scoped to `canonical.rs` specifically, per
  the incident that prompted it.
