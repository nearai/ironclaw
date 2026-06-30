//! HEADLINE scenario for Option P: two threads simultaneously parked on the
//! group's ONE shared `TurnCoordinator`, resolved independently with opposite
//! dispositions, asserting each run's own gate disposition was applied and
//! neither run's resume disturbed the other's pending gate or terminal state.
//!
//! Note on scope: the on-disk assertions below prove resume is dispatched
//! correctly by `run_id` (A's approval landed A's write; B's denial never
//! produced B's write). They do **not** prove per-thread workspace
//! isolation — `GroupSharedStorage` builds ONE `capability_recorder` /
//! workspace root for the whole group (`into_group`,
//! `tests/support/reborn/group.rs`), so `thread_a` and `thread_b` read and
//! write the *same* on-disk workspace. A file written under one thread's
//! handle is trivially visible from the other's, regardless of resume
//! correctness — that was confirmed empirically: an added
//! `thread_b.assert_workspace_file_absent("concurrent_a.txt")` check failed
//! because A's approved write is visible through B's handle on the shared
//! workspace. Cross-thread workspace isolation is therefore not a property
//! this group harness enforces or this scenario can test.
//!
//! ## Why this needs a new scenario (consolidate-don't-proliferate, CLAUDE.md
//! "Testing Discipline")
//!
//! `scenario_gate_then_approve` / `scenario_gate_then_deny` /
//! `scenario_approve_always_persists_cross_thread` each drive ONE thread's gate
//! to resolution *before* the next thread submits a turn — fully sequential.
//! Before this refactor, that was also the only coverage *possible*: each
//! group thread built its OWN `TurnRunScheduler` worker + coordinator, so two
//! "concurrent" gates would have lived on two independent schedulers and never
//! exercised shared dispatch state.
//!
//! Option P collapsed that to ONE coordinator/scheduler per group
//! (`GroupSharedStorage::coordinator`, built once in
//! `RebornIntegrationGroupBuilder::into_group`, `tests/support/reborn/group.rs`).
//! That is exactly the failure mode the original flake lived in: a global
//! worker pool claims runs off one shared queue, so a worker can in principle
//! pick up the wrong run for the wrong thread. The only way to prove resume
//! dispatch is keyed correctly (by `run_id`, not by registration order or a
//! shared scope) is to put TWO runs on the shared coordinator in `Blocked`
//! state AT THE SAME TIME — both turns in flight, both parked on a gate,
//! before either is resolved — then resolve them with DIFFERENT, divergent
//! dispositions and assert each thread's own real side effect. No existing
//! scenario can absorb this: doing it sequentially (as the three existing
//! scenarios do) cannot distinguish "resume keys on run_id" from "resume keys
//! on registration order" or "resume keys on a constant", because there is
//! never more than one blocked run on the coordinator at a time to confuse.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // Two distinct threads, two distinct gated writes, different target files
    // so each run's own disposition (A approved, B denied) is independently
    // verifiable on disk — not just from in-process return values. Both
    // threads share one on-disk workspace (see module note above), so this
    // proves resume is dispatched by `run_id` rather than proving per-thread
    // workspace isolation.
    let thread_a = g
        .thread("conv-concurrent-dual-gate-a")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/concurrent_a.txt", "content": "thread A approved"}),
            ),
            RebornScriptedReply::text("A: write approved"),
        ])
        .build()
        .await?;
    let thread_b = g
        .thread("conv-concurrent-dual-gate-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/concurrent_b.txt", "content": "thread B should not persist"}),
            ),
            RebornScriptedReply::text("B: write was not authorized"),
        ])
        .build()
        .await?;

    // Submit both turns and drive both to `BlockedApproval`, then resolve them
    // one at a time. The essential coverage is COEXISTENCE: two distinct runs
    // are simultaneously parked on the ONE shared coordinator (A stays blocked
    // while B is submitted and blocked), and each gate is then resolved by its
    // own `run_id`. We deliberately do NOT `tokio::join!` the submit/resume
    // calls: hammering the shared CAS-over-libsql turn-state store with two
    // *truly parallel* read-modify-write turns is a separate, prod-relevant
    // concurrency concern (see issue: parallel same-tenant runs vs the
    // `FilesystemTurnStateStore` CAS loop / libsql backend) that this
    // test-framework refactor is not the place to fix — and it is orthogonal to
    // what this scenario proves (run_id-keyed resume on a shared coordinator).
    let (run_a, gate_a) = thread_a
        .submit_turn_until_blocked("write the concurrent A file")
        .await?;
    let (run_b, gate_b) = thread_b
        .submit_turn_until_blocked("write the concurrent B file")
        .await?;
    // Both runs are now parked on the shared coordinator at the same time.

    // Non-vacuity: two genuinely distinct runs/gates are actually in flight —
    // if the harness collapsed both threads onto one run this would catch it
    // before the resolution step even starts.
    if run_a == run_b {
        return Err(format!("expected distinct run ids, both runs were {run_a}").into());
    }
    if gate_a.as_str() == gate_b.as_str() {
        return Err(format!("expected distinct gate refs, both were {gate_a:?}").into());
    }

    // Resolve independently with OPPOSITE dispositions. APPROVE A first and
    // drive it to completion while B is STILL blocked — if resume keyed on
    // anything other than `run_id` (a constant, or "most recently blocked
    // run"), approving A would disturb B's still-pending gate.
    thread_a.approve_gate(run_a, &gate_a).await?;
    let state_a = thread_a
        .wait_for_status(run_a, TurnStatus::Completed)
        .await?;

    // Now DENY B. Its gate must still be intact and independently resolvable.
    thread_b.deny_gate(run_b, &gate_b).await?;
    let state_b = thread_b
        .wait_for_status(run_b, TurnStatus::Completed)
        .await?;

    // No error-category leakage: a genuinely-correct concurrent resume reaches
    // `Completed` with no recorded failure on either run. Cross-resume bugs in
    // this shared-coordinator shape historically surface as
    // `driver_protocol_violation` (the masked failure category the de-mask fix
    // addresses) or `TraceLlm exhausted` (a worker draining the wrong thread's
    // scripted-reply deque) — assert neither leaked onto either run.
    for (label, state) in [("A", &state_a), ("B", &state_b)] {
        if let Some(failure) = &state.failure {
            return Err(format!(
                "thread {label} run reached Completed but recorded a failure \
                 (no error category should leak on a clean concurrent resume): {failure:?}"
            )
            .into());
        }
    }

    // A's gate was APPROVED: the gated capability re-dispatched and the real
    // write landed on disk under A's own path.
    thread_a
        .assert_workspace_file_contains("concurrent_a.txt", "thread A approved")
        .await?;
    // B's gate was DENIED: the gated capability was never re-dispatched — B's
    // file must be absent. If A's approval had cross-bled onto B's run (e.g.
    // resume dispatch ignoring `run_id`), this file would exist.
    thread_b
        .assert_workspace_file_absent("concurrent_b.txt")
        .await?;
    // The negative-space check in the other direction: B's content must never
    // have landed at all (rules out a resume path that writes to "whichever
    // gate resolved last" regardless of which run it came from). Checked via
    // `thread_a`'s handle, but recall both handles read the same shared
    // workspace (see module note above) — this is the same on-disk fact as
    // the `thread_b` check above, re-asserted for readability at the call
    // site, not an independent per-thread isolation proof.
    thread_a
        .assert_workspace_file_absent("concurrent_b.txt")
        .await?;

    Ok(())
}
