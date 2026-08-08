//! C-MULTIUSER scenario: per-actor run projection isolation over one shared
//! process journal. The negative assertions query another actor's process id
//! through the reader's own scope and require the journal projection to reject
//! it.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::{AgentTurnRuntimePort, GetRunStateRequest, TurnError, TurnRunId, TurnScope};

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (default actor): complete a turn over the shared turn_store ──
    let a = g
        .thread("conv-turnstate-iso-a")
        .script([RebornScriptedReply::text("reply-for-actor-a")])
        .build()
        .await?;
    let run_a = a
        .submit_turn("hello from actor a")
        .await
        .map_err(|e| format!("[A submit] {e}"))?;

    // ── Actor B (DISTINCT actor, SAME shared turn_store) ──────────────────────
    let b = g
        .thread("conv-turnstate-iso-b")
        .with_actor_id("reborn-actor-b")
        .script([RebornScriptedReply::text("reply-for-actor-b")])
        .build()
        .await?;
    // Non-vacuity: if `with_actor_id` regressed to a no-op, both scopes would
    // be identical and the negative pins below would trivially "pass".
    if a.binding.actor_user_id == b.binding.actor_user_id {
        return Err("with_actor_id seam no-op: both actors resolved the same owner".into());
    }
    let run_b = b
        .submit_turn("hello from actor b")
        .await
        .map_err(|e| format!("[B submit] {e}"))?;

    // Positive pins: each actor reads back its OWN run under its OWN scope,
    // and gets genuinely ITS record back (not just any `Ok(_)`).
    assert_own_run_state_readable(a.turn_runtime.as_ref(), a.turn_scope.clone(), run_a, "A")
        .await?;
    assert_own_run_state_readable(b.turn_runtime.as_ref(), b.turn_scope.clone(), run_b, "B")
        .await?;

    // Negative pins, SYMMETRIC both ways: the store's scope-equality gate must
    // reject a request for the OTHER actor's run_id under the reader's OWN
    // scope — the one mechanism actually enforcing isolation given the shared
    // physical snapshot file (see module doc).
    assert_cannot_read_other_run_state(
        a.turn_runtime.as_ref(),
        a.turn_scope.clone(),
        run_b,
        "A",
        "B",
    )
    .await?;
    assert_cannot_read_other_run_state(
        b.turn_runtime.as_ref(),
        b.turn_scope.clone(),
        run_a,
        "B",
        "A",
    )
    .await?;

    Ok(())
}

async fn assert_own_run_state_readable(
    turn_store: &impl AgentTurnRuntimePort,
    scope: TurnScope,
    run_id: TurnRunId,
    actor_name: &str,
) -> HarnessResult<()> {
    let state = turn_store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .map_err(|e| format!("[{actor_name} own get_run_state] {e}"))?;
    if state.run_id != run_id {
        return Err(format!(
            "[{actor_name} own get_run_state] returned run_id {} instead of {run_id}",
            state.run_id
        )
        .into());
    }
    Ok(())
}

async fn assert_cannot_read_other_run_state(
    turn_store: &impl AgentTurnRuntimePort,
    reader_scope: TurnScope,
    other_run_id: TurnRunId,
    reader_name: &str,
    other_name: &str,
) -> HarnessResult<()> {
    let result = turn_store
        .get_run_state(GetRunStateRequest {
            scope: reader_scope,
            run_id: other_run_id,
        })
        .await;
    match result {
        Err(TurnError::ScopeNotFound) => Ok(()),
        Err(other) => Err(format!(
            "isolation guard fired for the wrong reason: actor {reader_name} reading actor \
             {other_name}'s run_id under its own scope returned {other:?}, expected \
             TurnError::ScopeNotFound"
        )
        .into()),
        Ok(leaked) => Err(format!(
            "isolation failure: actor {reader_name} read actor {other_name}'s run state \
             ({leaked:?}) under its OWN scope"
        )
        .into()),
    }
}
