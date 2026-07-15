# Agent-Loop `canonical.rs` — No Inline Decision Logic Or Hand-Rolled Mechanics

This rule exists because the same class of drift shipped in two different
PRs, by two different contributors, in one sitting:

- PR #6013 (tools-capable completion nudge) added a `completion_nudge_should_fire()`
  check plus direct mutation of `stop_state.{completion_nudges_used,
  completion_nudge_pending, last_reply_trailed_off}` inline in `canonical.rs`'s
  `execute()` body, on the main per-iteration path only — the `ResumeApproval`/
  `ResumeAuth`/`ResumeExternalTool`/`SkipModel` paths hit the same match shape a
  hundred-plus lines further down but never got the check. Not a deliberate
  scope decision — a silent inconsistency from editing one of three
  near-identical blocks and missing the other two.
- PR #5487 ("Add inner latency spans") wrapped ~24 call sites across
  `canonical.rs` with a `latency::stage!` macro invocation, each repeating
  `host.run_context()` (already reachable via `ctx.host`) and a hardcoded
  operation-name string, plus two more call sites that hand-rolled a third,
  slightly different manual timing pattern instead of reusing the macro.

Neither addition was reviewed as "does this belong in a stage" — each looked
like a small, local diff at the time. `crates/ironclaw_agent_loop/CLAUDE.md`
already says "Put lifecycle mechanics in the owning executor stage instead of
adding branch logic directly to `canonical.rs`," and the existing prose didn't
catch either incident in review. This file exists to give reviewers a concrete
worked example and a mechanical review flag.

## The rule

Any new decision logic (a stop/nudge/retry/gate policy check, a threshold
comparison, a `state.*` mutation gated on a condition) or repeated
cross-cutting mechanics (timing, tracing, progress emission) goes into the
stage or method that already owns the related state or decision. It is never
appended inline to `canonical.rs`'s `execute()` body or its per-branch
`execute_*_turn` methods.

If no existing stage owns the new behavior, that absence is the signal a new
stage (or a new method on an existing stage) is needed — not a signal to
inline the logic where it's convenient to write.

`canonical.rs` is the ordered lifecycle spine: it dispatches to stages and
sequences their outputs. Its per-branch methods (`execute_prepared_turn`,
`execute_resume_turn`, `execute_skip_model_turn`) exist so each branch gets
its own stack frame and so a diff to one branch is visibly scoped to that
branch — they are not a looser license to inline stage-owned logic just
because the surrounding function is now smaller.

## Review flag

A PR diff to `canonical.rs` (or its `execute_*_turn` methods) that adds any
of the following, without a corresponding change in the file that already
owns the related stage/decision, should be rejected or asked to justify why
the logic doesn't belong there instead:

- a new `if`/`match` arm that reads or mutates a `state.*` field based on a
  condition (a policy/threshold check);
- a new hand-rolled timing block (`Instant::now()`/manual duration
  arithmetic) instead of the shared `latency::stage!` macro or a `timed`/
  `_timed` sibling method on the relevant stage;
- a new helper function defined in `canonical.rs` that only one of the three
  `execute_*_turn` methods calls (that's a sign the logic belongs inside that
  one method's owning stage, not as a free function in the spine file).

## Worked examples (what should have happened instead)

**Completion nudge (PR #6013):** the decision belongs to `StopStage`, the
single stage all three branches already call for the Stop-vs-Continue
decision — not to a bespoke check written inline. Landed fix: moved the
predicate into `loop_exit.rs` (co-located with the sibling nudge functions
and constants it already depends on), and added
`StopStage::decide_with_completion_nudge()` as a dedicated wrapper around the
existing `decide()`. Only `execute_prepared_turn` calls the wrapper;
`execute_resume_turn`/`execute_skip_model_turn` call plain `decide()`,
unmodified — the main-path-only behavior is now visible in *which method is
called*, not an accidental omission two call sites away.

**Latency spans (PR #5487):** the timing/tracing concern belongs to the
`ExecutorStage` trait (for `.process()`-shaped calls) or a `_timed` sibling
method defined next to the inherent method it wraps (for stages with extra
entry points, like `StopStage::observe`/`decide` or `CheckpointStage::
cancel_if_requested`) — not to 24 individual macro invocations at each call
site in the spine. Landed fix: `pipeline::timed()` (a generic function
wrapping `ExecutorStage::process`) plus `cancel_if_requested_timed`/
`emit_progress_timed` (`checkpoint.rs`), `observe_timed`/`decide_timed`/
`decide_with_completion_nudge_timed` (`turn_stop.rs`), and `ack_timed`
(`executor.rs`, next to `PendingInputAck::ack`).

**A near-miss that turned out to be correctly placed:** the repeated-call-
warning `driver_note` progress event and its paired
`state.stop_state.mark_repeated_call_warning_rendered()` mutation still live
in `execute_prepared_turn`, gated on `prompt.rendered_repeated_call_warning`.
This looks like the same anti-pattern as the two incidents above, but
relocating it into `PromptStage` (which computes the flag) would silently
flip `BeforeModel` checkpoint semantics: `canonical.rs` currently writes that
checkpoint *before* applying the mutation, so a resumed run's checkpoint
correctly observes `RepeatedCallWarningPhase::PendingRender`. `PromptStage`
runs *before* the checkpoint write, so if it owned the mutation too, the
checkpoint would observe `Rendered` instead — a real behavior change, caught
by the existing test
`repeated_call_warning_checkpoint_stays_pending_until_model_request`. Lesson:
before relocating a mutation into the stage that computes its trigger
condition, check whether another spine-owned ordering (a checkpoint write, an
ack, a cancellation boundary) depends on the mutation happening at its
current point in sequence. When it does, the mutation staying in the spine
is the correct call, not a violation of this rule — but leave a comment
explaining why, so a future reader doesn't "fix" it into a regression.

## What this rule does not cover

- Sequencing calls to stages and matching on their typed outputs — that *is*
  `canonical.rs`'s job.
- A `state.iteration = state.iteration.saturating_add(1)` or similar
  bookkeeping that belongs to the spine's own iteration protocol, not to any
  single stage's decision.
- The `IterationStarted` progress event, emitted once per iteration from the
  spine itself — it marks the top of the loop tick, which is the spine's own
  bookkeeping, not a stage's decision.

## References

- Incidents: PR #6013 (completion nudge), PR #5487 (latency spans).
- Design spec with the full incident writeup and fix:
  `docs/superpowers/specs/2026-07-14-agent-loop-canonical-slop-cleanup-design.md`.
- Sibling rule with the same shape (a class of drift that recurred until it
  got a named rule and a worked example): `agent-loop-capabilities.md`.
