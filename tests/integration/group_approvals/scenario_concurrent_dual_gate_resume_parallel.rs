//! Part 2a (#5466 / F-CAS-CONTENTION): real `tokio::join!` parallel pressure
//! against the group's ONE shared CAS turn-state store
//! (`FilesystemTurnStateRowStore` over `InMemoryBackend` -- the same concrete
//! CAS-over-`RootFilesystem` mechanism prod uses, independent of
//! `StorageMode`). Sibling of `scenario_concurrent_dual_gate_resume.rs`,
//! whose own module doc explicitly defers this: "truly parallel
//! read-modify-write turns against the shared CAS turn-state store is a
//! separate, prod-relevant concern orthogonal to what this scenario proves."
//!
//! #5466 measured ~10% of single-attempt real-parallel exchanges landing on
//! a sanitized `exit_application_failed` catch-all instead of `Completed`,
//! root-caused to `FilesystemTurnStateRowStore`'s lock-free CAS retry churning
//! a fresh libsql connection per attempt under concurrent contention (see
//! `crates/ironclaw_turns/src/filesystem_store.rs`'s `cas_update`). #5751
//! fixed the root cause with a bounded deadpool connection pool. Verified
//! here (cycle-3 fix lane, PR #5819): 50 real `StorageMode::LibSql` runs and
//! 40 `InMemory` runs, 0 failures and 0 tolerated-flake occurrences in
//! either -- both the libsql exclusion and the retry-tolerance this
//! scenario used to need are retired.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let path_a = "parallel_a.txt";
    let path_b = "parallel_b.txt";
    let content_a = "thread A approved (parallel)";
    let content_b = "thread B should not persist";

    let thread_a = g
        .thread("conv-concurrent-dual-gate-parallel-a")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": format!("/workspace/{path_a}"), "content": content_a}),
            ),
            RebornScriptedReply::text("A: write approved"),
        ])
        .build()
        .await?;
    let thread_b = g
        .thread("conv-concurrent-dual-gate-parallel-b")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": format!("/workspace/{path_b}"), "content": content_b}),
            ),
            RebornScriptedReply::text("B: write was not authorized"),
        ])
        .build()
        .await?;

    // Real parallel pressure (unlike the sequential sibling scenario): both
    // submits, both gate resolutions, and both terminal waits are
    // `tokio::join!`ed against the ONE shared CAS turn-state store.
    let (blocked_a, blocked_b) = tokio::join!(
        thread_a.submit_turn_until_blocked("write the concurrent parallel A file"),
        thread_b.submit_turn_until_blocked("write the concurrent parallel B file"),
    );
    let (run_a, gate_a) = blocked_a?;
    let (run_b, gate_b) = blocked_b?;

    // Non-vacuity: two distinct runs actually got parked simultaneously.
    if run_a == run_b {
        return Err(format!("expected distinct run ids, both runs were {run_a}").into());
    }
    if gate_a.as_str() == gate_b.as_str() {
        return Err(format!("expected distinct gate refs, both were {gate_a:?}").into());
    }

    let (approve_result, deny_result) = tokio::join!(
        thread_a.approve_gate(run_a, &gate_a),
        thread_b.deny_gate(run_b, &gate_b),
    );
    approve_result?;
    deny_result?;

    let (state_a, state_b) = tokio::join!(
        thread_a.wait_for_terminal(run_a),
        thread_b.wait_for_terminal(run_b),
    );
    let state_a = state_a?;
    let state_b = state_b?;

    for (label, state) in [("A", &state_a), ("B", &state_b)] {
        if state.status != TurnStatus::Completed {
            return Err(format!(
                "thread {label} expected Completed, got {:?}; state={state:?}",
                state.status
            )
            .into());
        }
        if let Some(failure) = &state.failure {
            return Err(format!(
                "thread {label} reached Completed but recorded a failure: {failure:?}"
            )
            .into());
        }
    }

    // No cross-thread bleed / no lost update: B (denied) must never have its
    // file; A's (approved) file must be present with its own content.
    thread_b.assert_workspace_file_absent(path_b).await?;
    thread_a.assert_workspace_file_absent(path_b).await?;
    thread_a
        .assert_workspace_file_contains(path_a, content_a)
        .await?;

    Ok(())
}
