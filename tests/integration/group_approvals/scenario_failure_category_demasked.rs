//! Scenario: a genuinely-FAILED group run reports its TRUE failure category,
//! not the masking `driver_protocol_violation`.
//!
//! `IronClawIntegrationGroupBuilder::into_group` (`tests/integration/support/group.rs`
//! ~line 472) wires `.with_checkpoint_state_store(..)` onto the group-level
//! `ThreadCheckpointLoopExitEvidencePort` -- the de-mask fix. Without it,
//! `verify_failure_evidence` (`crates/ironclaw_runner/src/loop_exit_applier.rs`)
//! short-circuits `Ok(false)` on a `None` store, so `validate_failed_exit`
//! (`crates/ironclaw_turns/src/loop_exit.rs`) rewrites every `Failed` exit to
//! the opaque `"driver_protocol_violation"` category. With the store wired, a
//! verified failed exit's TRUE category survives onto `TurnRunState::failure`.
//!
//! New scenario needed: every other scenario in this binary asserts clean
//! completion/gate resolution and never drives a run to `TurnStatus::Failed`,
//! so none exercises `verify_failure_evidence`; the `loop_exit_applier` unit
//! tests cover the fixture but not `group.rs`'s end-to-end wiring.
//!
//! Failure is produced deterministically: an EMPTY scripted-reply list
//! (`.script([])`) makes `TraceLlm::next_step` return `LlmError::RequestFailed`
//! on the first model call; once the bounded-retry `RecoveryStrategy` budget
//! is exhausted, the loop emits `LoopExit::Failed`. Under the provider-error
//! fidelity mapping (`ModelErrorClass::Unavailable`), an exhausted gateway
//! that cannot serve the call surfaces as category `"model_unavailable"`.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use ironclaw_turns::TurnStatus;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // No scripted replies: the first model call exhausts the scripted
    // provider, deterministically driving the run to `Failed` (see module doc).
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

    // The de-mask fix's entire point (see module doc): the TRUE category
    // must survive, not the masking sentinel.
    if failure.category() == "driver_protocol_violation" {
        return Err(format!(
            "failure category was the masking sentinel \"driver_protocol_violation\"; \
             the group-level checkpoint_state_store wiring (group.rs ~line 472) is not \
             de-masking the real failure category (got: {failure:?})"
        )
        .into());
    }

    // Empirically confirmed: `TraceLlm` exhaustion surfaces as
    // `ModelErrorClass::Unavailable` -> category `"model_unavailable"`
    // (`crates/ironclaw_agent_loop/src/executor/mapping.rs`). Asserting the
    // exact value proves the de-masked path produces the loop's REAL reason,
    // not just any non-sentinel category.
    if failure.category() != "model_unavailable" {
        return Err(format!(
            "expected de-masked failure category \"model_unavailable\" (TraceLlm exhaustion -> \
             ModelErrorClass::Unavailable), got {failure:?}"
        )
        .into());
    }

    Ok(())
}
