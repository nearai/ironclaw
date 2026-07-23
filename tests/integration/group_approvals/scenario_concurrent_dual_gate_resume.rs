//! HEADLINE scenario for Option P: two threads simultaneously parked on the
//! group's ONE shared `TurnCoordinator`, resolved independently with opposite
//! dispositions, asserting resume is dispatched by `run_id` and neither run's
//! resume disturbs the other's pending gate or terminal state.
//!
//! Scope: `GroupSharedStorage` gives the whole group ONE workspace root, so
//! this does NOT prove per-thread workspace isolation — confirmed
//! empirically: an added `thread_b.assert_workspace_file_absent("concurrent_a.txt")`
//! check failed because A's write is visible through B's handle on the
//! shared workspace. Only resume-by-`run_id` is asserted here.
//!
//! New scenario needed (consolidate-don't-proliferate, CLAUDE.md "Testing
//! Discipline"): the sibling gate scenarios resolve one thread's gate before
//! the next submits, so at most one run is ever `Blocked` at a time — that
//! can't distinguish "resume keys on `run_id`" from "resume keys on
//! registration order". This scenario puts two runs in `Blocked` state on
//! the shared coordinator SIMULTANEOUSLY, then resolves them with opposite
//! dispositions.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // Two distinct threads, two distinct gated writes, different target files
    // so each run's own disposition (A approved, B denied) is independently
    // verifiable on disk (see module note on shared-workspace scope).
    let thread_a = g
        .thread("conv-concurrent-dual-gate-a")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/concurrent_a.txt", "content": "thread A approved"}),
            ),
            IronClawScriptedReply::text("A: write approved"),
        ])
        .build()
        .await?;
    let thread_b = g
        .thread("conv-concurrent-dual-gate-b")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/concurrent_b.txt", "content": "thread B should not persist"}),
            ),
            IronClawScriptedReply::text("B: write was not authorized"),
        ])
        .build()
        .await?;

    // Submit both turns and drive both to `BlockedApproval`, then resolve them
    // one at a time. The essential coverage is COEXISTENCE: two distinct runs
    // are simultaneously parked on the ONE shared coordinator, each then
    // resolved by its own `run_id`. We deliberately do NOT `tokio::join!` the
    // submit/resume calls -- truly parallel read-modify-write turns against
    // the shared CAS turn-state store is a separate, prod-relevant concern
    // orthogonal to what this scenario proves.
    let (run_a, gate_a) = thread_a
        .submit_turn_until_blocked("write the concurrent A file")
        .await?;
    let (run_b, gate_b) = thread_b
        .submit_turn_until_blocked("write the concurrent B file")
        .await?;

    // Non-vacuity: catches the harness collapsing both threads onto one run
    // before the resolution step even starts.
    if run_a == run_b {
        return Err(format!("expected distinct run ids, both runs were {run_a}").into());
    }
    if gate_a.as_str() == gate_b.as_str() {
        return Err(format!("expected distinct gate refs, both were {gate_a:?}").into());
    }

    // Resolve independently with OPPOSITE dispositions. APPROVE A first while
    // B is STILL blocked -- if resume keyed on anything other than `run_id`,
    // approving A would disturb B's still-pending gate.
    thread_a.approve_gate(run_a, &gate_a).await?;
    let state_a = thread_a
        .wait_for_status(run_a, TurnStatus::Completed)
        .await?;

    // Now DENY B. Its gate must still be intact and independently resolvable.
    thread_b.deny_gate(run_b, &gate_b).await?;
    let state_b = thread_b
        .wait_for_status(run_b, TurnStatus::Completed)
        .await?;

    // No error-category leakage: cross-resume bugs here historically surface
    // as `driver_protocol_violation` or `TraceLlm exhausted` (wrong thread's
    // scripted-reply deque drained) -- assert neither leaked onto either run.
    for (label, state) in [("A", &state_a), ("B", &state_b)] {
        if let Some(failure) = &state.failure {
            return Err(format!(
                "thread {label} run reached Completed but recorded a failure \
                 (no error category should leak on a clean concurrent resume): {failure:?}"
            )
            .into());
        }
    }

    // A's gate was APPROVED: the write landed on disk under A's own path.
    thread_a
        .assert_workspace_file_contains("concurrent_a.txt", "thread A approved")
        .await?;
    // B's gate was DENIED: its file must be absent -- would exist if A's
    // approval cross-bled onto B's run.
    thread_b
        .assert_workspace_file_absent("concurrent_b.txt")
        .await?;
    // Re-checked via `thread_a`'s handle (shared workspace): rules out a
    // resume path keyed on "whichever gate resolved last" instead of run_id.
    thread_a
        .assert_workspace_file_absent("concurrent_b.txt")
        .await?;

    Ok(())
}
