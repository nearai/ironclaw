# Agent-loop canonical.rs slop cleanup (not a bug — refactor)

Base: origin/main (verified no further ironclaw_agent_loop changes since b0b268da6)
Spec: docs/superpowers/specs/2026-07-14-agent-loop-canonical-slop-cleanup-design.md (committed on main, main branch already had it before worktree creation)
Worktree branch: worktree-agent-loop-canonical-cleanup

## Scope (3 sections, behavior-preserving)
1. Completion-nudge: move `completion_nudge_should_fire` predicate into loop_exit.rs
   (co-located with COMPLETION_NUDGE_LIMIT + sibling nudge fns; matches existing
   budget.rs -> loop_exit::try_final_answer_nudge precedent). Add NEW
   `StopStage::decide_with_completion_nudge()` (turn_stop.rs) that wraps the
   existing, UNCHANGED `decide()` and applies the nudge check + stop_state mutation
   only when it returns Stop. Main path calls the new wrapper; resume/skip-model
   paths call plain `decide()` unchanged (zero diff at those 2 call sites — no bool
   flag threaded through StopInput). Reuse existing StopStep::Continue, no new variant.
   [Revised post-thermo-review: dropped the allow_completion_nudge:bool-on-StopInput
   design — it forced 2/3 call sites to pass a constant false. Wrapper method instead.]
2. Latency: add ExecutorStage::timed() default method (pipeline.rs) for trait
   .process() call sites; add _timed sibling inherent methods for
   observe/decide/cancel_if_requested/write/emit_progress/ack in their owning files.
3. Event streaming: move repeated-call-warning mark+emit from canonical.rs into
   PromptStage::process (prompt.rs). Leave IterationStarted call in canonical.rs,
   fold its timing into the Section 2 helper.

## Pre-verified test coverage
- completion_nudge_lets_model_use_tools_to_finish_after_trailing_off (main, gate on)
- completion_nudge_disabled_leaves_trailed_off_run_without_tool_use (main, gate off)
- completion_nudge_skipped_on_clean_reply (main, clean reply)
- repeated_call_warning_checkpoint_stays_pending_until_model_request (state+prompt content)
- safety_nets.rs repeated_call_warning_prompt_count call sites (x3)
- GAP (new test needed): resume/skip-model paths must never nudge (allow_completion_nudge=false)
- host.progress_events() exists on MockAgentLoopDriverHost — usable to assert
  driver_note event still fires after PromptStage move.

## Non-goals
- 3x duplicated observe->decide->exit structural duplication across main/resume/skip-model
- capabilities.rs / capability_helpers.rs untouched
- New .claude/rules/agent-loop-canonical-branching.md guardrail doc (separate follow-up,
  not blocking this PR)

## OUTCOME (2026-07-14)
- Section 1 SHIPPED: decide_with_completion_nudge wrapper, loop_exit.rs owns predicate,
  new StopStage-level regression test. 400/400 tests green, clippy clean, arch test green.
  canonical.rs 629 -> 573 lines.
- Section 2 ABANDONED: reproducible stack overflow in
  policy_denied_capability_error_honors_retry_recovery, localized (via print instrumentation)
  to inside CapabilityStage::process() (untouched code, called via original invocation).
  Ruled out: incremental-compile cache corruption (clean rebuild), async_trait default-method
  fragility (converted to free fn, still broke), Input:Send+'static trait bound. Root cause
  NOT FOUND despite extensive bisection. lldb blocked by macOS entitlements even with sandbox
  disabled. Reverted entirely, not shipped.
- Section 3 NOT ATTEMPTED but a real ordering hazard was found and documented: moving the
  repeated-call-warning mutation into PromptStage changes BeforeModel checkpoint semantics
  (PendingRender -> Rendered), caught by existing test
  repeated_call_warning_checkpoint_stays_pending_until_model_request. Any retry must preserve
  checkpoint-write ordering.
- Section 4 (guardrail rule file) NOT ATTEMPTED — time went to the Section 2 investigation.
- Spec doc updated in-place with this outcome (docs/superpowers/specs/2026-07-14-*.md).
- Working tree in worktree branch is clean, all changes committed-ready but NOT yet committed
  (paused to report to user before commit/PR per significant scope deviation from plan).
