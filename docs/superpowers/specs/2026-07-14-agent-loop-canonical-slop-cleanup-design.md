# Agent-Loop `canonical.rs` Slop Cleanup — Design

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

## Section 1 — Completion-nudge extraction

**Owner:** `StopStage::decide()` (`executor/turn_stop.rs`). Not the
`StopConditionStrategy` trait (`strategies/stop.rs`) — that trait's
methods take only `&LoopExecutionState`, with no access to `ctx.host` /
`SteeringPolicy`, so it structurally cannot make this decision.
`StopStage::decide()` already has `ctx.host` and is the single call site
all three paths (main, resume, skip-model) go through for the
Stop-vs-Continue decision.

Changes:

- Add `allow_completion_nudge: bool` to `StopInput`. Explicit per call
  site — the main per-iteration path passes `true`; the
  `ResumeApproval`/`SkipModel` paths pass `false`. This preserves today's
  exact behavior (main-path-only nudging) but makes it a visible,
  intentional choice at each call site instead of an accidental omission.
- Move `completion_nudge_should_fire()` (and the `stop_state` mutation
  currently inline in `canonical.rs`) into `turn_stop.rs`, called from
  inside `decide()` only when `input.allow_completion_nudge` is `true`
  and `StopOutcome::Stop { kind }` was returned.
- When the nudge fires, `decide()` returns the **existing**
  `StopStep::Continue { state, pending_input_ack }` variant — no new
  variant needed. `canonical.rs`'s existing `Continue` arm already does
  exactly `state = next; pending_input_ack = ack;`, which matches the
  current nudge branch's behavior.
- Net effect on `canonical.rs`: the `StopStep::Stop` arm returns to being
  a plain "call `self.exit.process`, ack, return" block — identical shape
  across all three paths, with no nudge-awareness left in canonical.rs.

## Section 2 — Latency-instrumentation extraction

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

This section is a pure mechanical extraction — no behavior change, no new
types, only where the timing/tracing code lives.

## Section 3 — Event-streaming cleanup

- Move the repeated-call-warning block (`state.stop_state
  .mark_repeated_call_warning_rendered()` + the `driver_note` progress
  emission) out of `canonical.rs` and into `PromptStage::process`
  (prompt.rs), right where `rendered_repeated_call_warning` is already
  computed. Deletes the block from `canonical.rs` entirely; no new types.
- `IterationStarted` emission stays a `canonical.rs` call — it marks the
  top of each loop tick, which is genuinely the spine's own bookkeeping,
  not any single stage's decision. Its hand-rolled timing block is folded
  into Section 2's `timed` cleanup so there's one timing style crate-wide
  instead of three.

## Section 4 — Guardrail: new rule file

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

- **Nudge (Section 1):** #6013's existing WITH/WITHOUT-gate pair through
  `CanonicalAgentLoopExecutor` remains the regression test for the main
  path. Add a new test asserting the `ResumeApproval`/`SkipModel` paths
  never nudge (i.e. `allow_completion_nudge: false` is honored) — this
  distinction was previously untestable since it was an accidental
  omission, not an explicit contract.
- **Latency (Section 2):** pure extraction, no behavior change; existing
  tests are the regression net. Add one unit test on the trait's `timed`
  default method confirming it still only emits a span when the
  `ironclaw_latency` trace target is enabled (same gate `started_at()`
  already implements).
- **Event streaming (Section 3):** existing prompt-stage tests already
  cover `rendered_repeated_call_warning` computation
  (`plan_context_request_suppresses_rendered_repeated_call_warning` in
  strategies/context.rs); extend/add a `PromptStage`-level test asserting
  the progress event + state mutation now fire from within
  `PromptStage::process` rather than relying on the moved canonical.rs
  behavior implicitly.
- All changes in this spec are internal refactors of already-tested
  lifecycle mechanics — no new production behavior is being added, so the
  bar is "prove behavior is unchanged," not "add new coverage for new
  behavior."

## Non-Goals (explicitly out of scope)

- **The 3x duplicated `stop.observe` → `stop.decide` → `exit` sequence**
  across the main/`ResumeApproval`/`SkipModel` paths in `canonical.rs` is
  a separate, larger structural question (the three paths differ in what
  happens *between* observe and decide — e.g. only the main path drains
  follow-up input) and is not addressed here. Section 2 shrinks its
  line-count footprint but does not collapse the duplication itself.
- No changes to `strategies/stop.rs`, `strategies/CLAUDE.md`, or any
  public strategy trait signature.
- No changes to `capabilities.rs`, `capability_helpers.rs`, or other large
  executor files — this spec is scoped to `canonical.rs` specifically, per
  the incident that prompted it.
