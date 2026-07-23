//! C-DENYEDGE rows 7 & 10: gate-ref edge cases the happy-path
//! `approve_gate`/`deny_gate` helpers cannot reach because they always
//! resolve the local-dev approval AND resume the coordinator with the SAME
//! `GateRef`.
//!
//! - `stale_gate_ref_resume` (row 7): the LOCAL-DEV approval resolve succeeds
//!   (using the run's real, correct gate_ref) but the COORDINATOR resume is
//!   issued with a different, stale `GateRef` — reaching
//!   `resume_turn_once`'s `record.gate_ref != Some(&request.gate_resolution_ref)`
//!   check (`TurnError::InvalidRequest { reason: "gate resolution reference
//!   mismatch" }`), distinct from a bogus ref failing earlier inside
//!   `approve_local_dev_gate`'s own request-id lookup.
//! - `missing_gate_bare_resolve` (row 10): a syntactically well-formed but
//!   never-issued `GateRef` is resolved on a thread that never raised any
//!   gate — pins the harness's own request-not-found rejection.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::{GateRef, TurnStatus};
use serde_json::json;

pub async fn stale_gate_ref_resume(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-stale-gate-ref")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/stale_ref.txt", "content": "stale ref write"}),
            ),
            IronClawScriptedReply::text("file written after approval"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the stale-ref file")
        .await?;

    // A syntactically valid but WRONG gate ref, distinct from the run's real,
    // still-blocked `gate_ref`.
    let stale_gate_ref = GateRef::new("gate:approval-00000000-0000-0000-0000-000000000000")
        .expect("valid bounded gate ref string");
    if gate_ref.as_str() == stale_gate_ref.as_str() {
        return Err("the stale ref fixture must not coincidentally match the real gate ref".into());
    }

    // Resolve the REAL approval (succeeds), but resume with the STALE ref.
    let err = h
        .approve_gate_with_stale_resume_ref(run_id, &gate_ref, &stale_gate_ref)
        .await
        .err()
        .ok_or("expected err: resuming with a stale gate ref must fail")?;
    let err_text = err.to_string();
    if !err_text.contains("gate resolution reference mismatch") {
        return Err(
            format!("expected the InvalidRequest gate-mismatch reason, got: {err_text}").into(),
        );
    }

    // Non-vacuity: the run is STILL blocked (stale-ref resume never cleared
    // the gate); resuming with the REAL gate ref completes it normally.
    h.resume_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;
    h.assert_workspace_file_contains("stale_ref.txt", "stale ref write")
        .await?;
    Ok(())
}

pub async fn missing_gate_bare_resolve(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-missing-gate")
        .script([IronClawScriptedReply::text("no tools needed here")])
        .build()
        .await?;

    // No tool call, so no gate is ever raised on this thread.
    let run_id = h.submit_turn("just say hello, no tools").await?;

    // Syntactically well-formed but NEVER-ISSUED gate ref: no capability call
    // on this thread recorded this request id, so `approve_gate`'s local-dev
    // resolve fails on the lookup before `resume_run` is ever reached.
    let bogus_gate_ref = GateRef::new("gate:approval-11111111-1111-1111-1111-111111111111")
        .expect("valid bounded gate ref string");

    let err = h
        .approve_gate(run_id, &bogus_gate_ref)
        .await
        .err()
        .ok_or("expected err: resolving a never-issued gate ref must fail")?;
    let err_text = err.to_string();
    if !err_text.contains("approval gate was not recorded by the host runtime harness") {
        return Err(format!(
            "expected the harness's own request-not-found rejection, got: {err_text}"
        )
        .into());
    }
    Ok(())
}
