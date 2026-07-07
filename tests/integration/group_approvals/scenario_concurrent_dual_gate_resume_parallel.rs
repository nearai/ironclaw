//! Part 2a (#5466 / F-CAS-CONTENTION): real `tokio::join!` parallel pressure
//! against the group's ONE shared CAS turn-state store
//! (`FilesystemTurnStateStore` over `InMemoryBackend` -- the same concrete
//! CAS-over-`RootFilesystem` mechanism prod uses, independent of
//! `StorageMode`). Sibling of `scenario_concurrent_dual_gate_resume.rs`,
//! whose own module doc explicitly defers this: "truly parallel
//! read-modify-write turns against the shared CAS turn-state store is a
//! separate, prod-relevant concern orthogonal to what this scenario proves."
//!
//! #5466 measured ~10% of single-attempt real-parallel exchanges landing on
//! a sanitized `exit_application_failed` catch-all (the `LoopExitApplier`
//! error-path category) instead of `Completed`. This is the one OBSERVED
//! symptom, category-level only -- NOT a root-cause diagnosis; the allow-list
//! below only proves this test doesn't silently swallow a DIFFERENT failure.
//!
//! DO NOT add a `StorageMode::LibSql` variant: #5466 reports the libsql
//! variant SIGABRTs the whole test process (`SQLITE_MISUSE` across an
//! `extern "C"` boundary) -- a process abort that would take every other
//! test in this `[[test]]` binary down with it. Exclude unconditionally
//! until #5466's libsql diagnosis lands a real fix.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::{TurnRunState, TurnStatus};
use serde_json::json;

/// #5466's own measured single-attempt failure rate is ~10%; this pushes the
/// compound false-fail rate to ~0.1%.
const MAX_ATTEMPTS: u32 = 3;
/// The one category #5466 reports observing. See module doc: category-level
/// allow-list only, not a root-cause diagnosis.
const ALLOWED_FAILURE_CATEGORY: &str = "exit_application_failed";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    for attempt in 1..=MAX_ATTEMPTS {
        if attempt_once(g, attempt).await? {
            return Ok(());
        }
    }
    Err(format!(
        "exhausted {MAX_ATTEMPTS} attempts; every attempt hit the allow-listed \
         {ALLOWED_FAILURE_CATEGORY} category"
    )
    .into())
}

/// One run's classified terminal outcome: either it completed cleanly, or it
/// hit the one tolerated flake category (#5466) and this attempt should retry.
enum RunOutcome {
    Completed,
    ToleratedFlake,
}

/// Classifies a terminal `TurnRunState`. `Err` for anything outside
/// Completed/allow-listed-Failed -- a genuine, non-tolerated regression.
fn classify_terminal(label: &str, state: &TurnRunState) -> HarnessResult<RunOutcome> {
    match state.status {
        TurnStatus::Completed => {
            if let Some(failure) = &state.failure {
                return Err(format!(
                    "thread {label} reached Completed but recorded a failure: {failure:?}"
                )
                .into());
            }
            Ok(RunOutcome::Completed)
        }
        TurnStatus::Failed => {
            let category = state.failure.as_ref().map(|f| f.category());
            if category != Some(ALLOWED_FAILURE_CATEGORY) {
                return Err(format!(
                    "thread {label} reached Failed with an unrecognized category {category:?} \
                     (only {ALLOWED_FAILURE_CATEGORY:?} is allow-listed); state={state:?}"
                )
                .into());
            }
            Ok(RunOutcome::ToleratedFlake)
        }
        other => Err(format!(
            "thread {label} reached unexpected terminal status {other:?} (expected \
             Completed or allow-listed Failed); state={state:?}"
        )
        .into()),
    }
}

/// Runs one full parallel exchange. `Ok(true)` on a clean pass, `Ok(false)`
/// if either run hit the tolerated flake (caller retries), `Err` otherwise.
async fn attempt_once(g: &RebornIntegrationGroup, attempt: u32) -> HarnessResult<bool> {
    let path_a = format!("parallel_a_{attempt}.txt");
    let path_b = format!("parallel_b_{attempt}.txt");
    let content_a = "thread A approved (parallel)";
    let content_b = "thread B should not persist";

    let thread_a = g
        .thread(format!("conv-concurrent-dual-gate-parallel-a-{attempt}"))
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
        .thread(format!("conv-concurrent-dual-gate-parallel-b-{attempt}"))
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

    // Bounded retry / no permanent wedge: an `Err` here (timeout) is always
    // fatal -- #5466's symptom is a fast terminal Failed, never a hang.
    let (state_a, state_b) = tokio::join!(
        thread_a.wait_for_terminal(run_a),
        thread_b.wait_for_terminal(run_b),
    );
    let state_a = state_a?;
    let state_b = state_b?;

    let outcome_a = classify_terminal("A", &state_a)?;
    let outcome_b = classify_terminal("B", &state_b)?;

    // No cross-thread bleed / no lost update, regardless of terminal status:
    // B (denied) must never have its file; A's file must match ITS OWN
    // disposition (present+correct only when Completed, absent when the
    // tolerated flake fired -- never B's content either way).
    thread_b.assert_workspace_file_absent(&path_b).await?;
    thread_a.assert_workspace_file_absent(&path_b).await?;
    match outcome_a {
        RunOutcome::Completed => {
            thread_a
                .assert_workspace_file_contains(&path_a, content_a)
                .await?;
        }
        RunOutcome::ToleratedFlake => {
            thread_a.assert_workspace_file_absent(&path_a).await?;
        }
    }

    if matches!(outcome_a, RunOutcome::ToleratedFlake)
        || matches!(outcome_b, RunOutcome::ToleratedFlake)
    {
        eprintln!(
            "w6-cas-contention: tolerated #5466 flake on attempt {attempt}/{MAX_ATTEMPTS}: \
             {ALLOWED_FAILURE_CATEGORY:?}"
        );
        return Ok(false);
    }
    Ok(true)
}
