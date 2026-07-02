//! Scenario: a genuinely-FAILED group run reports its TRUE failure category,
//! not the masking `driver_protocol_violation`.
//!
//! `RebornIntegrationGroupBuilder::into_group` (`tests/support/reborn/group.rs`,
//! ~line 472) wires `.with_checkpoint_state_store(checkpoint_state_store.clone())`
//! onto the group-level `ThreadCheckpointLoopExitEvidencePort` — that is the
//! de-mask fix. Without it, `ThreadCheckpointLoopExitEvidencePort::verify_failure_evidence`
//! (`crates/ironclaw_reborn/src/loop_exit_applier.rs`) returns `Ok(false)`
//! unconditionally (it short-circuits on `self.checkpoint_state_store` being
//! `None`), so `LoopExitApplier::apply`'s `validate_failed_exit`
//! (`crates/ironclaw_turns/src/loop_exit.rs`) treats every `Failed` exit as
//! `LoopExitViolationKind::UnverifiedFailureEvidence` and rewrites it to the
//! opaque `"driver_protocol_violation"` category, discarding the loop's real
//! failure reason. With the store wired, a failed exit whose checkpoint
//! actually recorded the claimed failure kind verifies and the run's TRUE
//! category survives onto `TurnRunState::failure`.
//!
//! ## Why this needs a NEW scenario (consolidate-don't-proliferate, root
//! CLAUDE.md "Testing Discipline")
//!
//! Every other scenario in this binary (`scenario_gate_then_approve`,
//! `scenario_gate_then_deny`, `scenario_concurrent_dual_gate_resume`,
//! `scenario_approve_always_persists_cross_thread`) asserts CLEAN completion
//! or gate resolution — none of them drives a run all the way to
//! `TurnStatus::Failed`, so none of them can observe the de-mask wiring at
//! all: `verify_failure_evidence` is only ever called on `LoopExit::Failed`.
//! `concurrent_dual_gate_resume`'s module doc explicitly calls out
//! `driver_protocol_violation` as a failure mode it asserts AGAINST (no
//! failure leaks on a clean run) — it cannot also be the test that proves the
//! masked category is correctly de-masked on a genuinely failed run, because
//! it never produces one. The `loop_exit_applier` unit tests
//! (`crates/ironclaw_reborn/src/loop_exit_applier/tests/mod.rs`) exercise
//! `verify_failure_evidence` directly against a hand-built fixture; they do
//! NOT prove the group composition in `group.rs` actually wires the store
//! through `into_group` end-to-end over the real scheduler/coordinator. This
//! scenario is the only one that closes that gap.
//!
//! ## How the failure is produced
//!
//! The thread is built with an EMPTY scripted-reply list
//! (`.script([])`). `TraceLlm::next_step` (`tests/support/trace_llm.rs`)
//! returns `LlmError::RequestFailed` ("TraceLlm exhausted: served 0 call(s),
//! no steps left") on the very first model call — deterministic, no tool
//! dispatch or timing race involved. That model error flows through the
//! bounded-retry `RecoveryStrategy` (`crates/ironclaw_agent_loop/src/strategies/recovery.rs`,
//! default `max_attempts_per_class: 2`) until the retry budget is exhausted,
//! at which point the loop emits `LoopExit::Failed` with
//! `LoopFailureKind::ModelError`, attaches the more specific safe summary
//! category `"model_unavailable"`, and the run reaches `TurnStatus::Failed`.
//! This is exactly the original flake's failure shape (model/driver failure
//! racing checkpoint evidence), now correctly surfaced through the de-masked
//! path instead of being swallowed into `driver_protocol_violation`.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use ironclaw_turns::TurnStatus;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // No scripted replies at all: the very first model call exhausts the
    // scripted provider, deterministically driving the run to a genuine
    // `Failed` terminal state (see module doc).
    let h = g
        .thread("conv-failure-category-demasked")
        .script([])
        .build()
        .await?;

    let run_id = h.submit_turn_async("trigger a model failure").await?;
    let state = h.wait_for_status(run_id, TurnStatus::Failed).await?;

    let failure = state
        .failure
        .as_ref()
        .ok_or("run reached Failed but TurnRunState::failure was None")?;

    // The de-mask fix's entire point: the TRUE failure category must survive,
    // not the masking sentinel that `UnverifiedFailureEvidence` rewrites
    // every unverified `Failed` exit to.
    if failure.category() == "driver_protocol_violation" {
        return Err(format!(
            "failure category was the masking sentinel \"driver_protocol_violation\"; \
             the group-level checkpoint_state_store wiring (group.rs ~line 472) is not \
             de-masking the real failure category (got: {failure:?})"
        )
        .into());
    }

    // A `TraceLlm` exhaustion on the first model call is classified as
    // `ModelErrorClass::Unavailable`. The loop-level `reason_kind` remains
    // `LoopFailureKind::ModelError`, while the persisted `TurnRunState::failure`
    // uses the more specific safe summary category from the failed exit.
    // Asserting the exact value (not just `!= driver_protocol_violation`)
    // proves the de-masked path preserves the loop's real safe category, not
    // some other incidental category.
    if failure.category() != "model_unavailable" {
        return Err(format!(
            "expected de-masked failure category \"model_unavailable\" (TraceLlm exhaustion -> \
             ModelErrorClass::Unavailable safe summary), got {failure:?}"
        )
        .into());
    }

    Ok(())
}
