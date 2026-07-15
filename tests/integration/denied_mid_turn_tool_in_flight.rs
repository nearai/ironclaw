//! Compound fault-injection scenario: a capability call that already
//! completed and persisted a real side effect survives a LATER gate denial
//! on the same conversation, rather than being rolled back, corrupted, or
//! masked.
//!
//! Real path (`RebornIntegrationGroup::live_approvals`, the same real gate
//! path `scenario_gate_then_approve.rs`/`scenario_gate_then_deny.rs` drive,
//! composed across two turns on one thread): turn 1 scripts a
//! `builtin.write_file` call, approved -- a real, persisted, "already in
//! flight" side effect from earlier in the SAME conversation. Turn 2 scripts
//! a DIFFERENT `builtin.write_file` call, denied. `deny_gate` resumes the run
//! so the executor surfaces a non-retryable authorization failure for turn
//! 2's write; turn 1's file must remain exactly as it was.
//!
//! Closes a gap `scenario_gate_then_deny.rs` doesn't cover: that scenario's
//! thread has no prior committed capability side effect, so nothing proves a
//! LATER denial leaves an EARLIER approved write untouched. (An earlier
//! design tried this within a single model step -- one scripted
//! `tool_calls([..])` batch mixing an ungated `read_file` with a gated
//! `write_file` -- but empirically, resuming a gate raised by the
//! non-first call in a mixed batch does not re-dispatch the gated capability
//! at all, approved or denied; that path needs its own investigation and is
//! out of scope here. The two-turn shape below uses only the proven
//! single-call-per-turn gate/resume mechanism each of those two scenarios
//! already exercises independently.)

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_turns::TurnStatus;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const ALREADY_IN_FLIGHT_CONTENT: &str = "already-in-flight approved content";

#[tokio::test]
async fn approved_write_survives_a_later_denied_gate_on_the_same_thread() {
    let g = RebornIntegrationGroup::live_approvals()
        .await
        .expect("group builds");
    let h = g
        .thread("conv-denied-mid-turn-tool-in-flight")
        .script([
            // Turn 1: approved write -- the "already in flight" committed
            // side effect from earlier in this conversation.
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/already-in-flight.txt", "content": ALREADY_IN_FLIGHT_CONTENT}),
            ),
            RebornScriptedReply::text("first file written"),
            // Turn 2: a DIFFERENT write, denied.
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/denied.txt", "content": "should not persist"}),
            ),
            RebornScriptedReply::text("the second write was not authorized"),
        ])
        .build()
        .await
        .expect("harness builds");

    // Turn 1: approve. Real, persisted side effect -- not scripted/faked.
    let (run1, gate1) = h
        .submit_turn_until_blocked("write the first file")
        .await
        .expect("blocked on the first write's approval gate");
    h.approve_gate(run1, &gate1)
        .await
        .expect("first gate approved");
    h.wait_for_status(run1, TurnStatus::Completed)
        .await
        .expect("first turn completes");
    h.assert_workspace_file_contains("already-in-flight.txt", ALREADY_IN_FLIGHT_CONTENT)
        .await
        .expect("first write persisted");

    // Turn 2: deny. A DIFFERENT gate, on the SAME thread, after the first
    // write already completed.
    let (run2, gate2) = h
        .submit_turn_until_blocked("write the second file")
        .await
        .expect("blocked on the second write's approval gate");
    h.deny_gate(run2, &gate2).await.expect("second gate denied");
    h.wait_for_status(run2, TurnStatus::Completed)
        .await
        .expect("run reaches Completed after the denial (not hung, not Failed)");

    // The denied write must NOT have executed.
    h.assert_workspace_file_absent("denied.txt")
        .await
        .expect("the denied write never ran");

    // The compound-failure invariant under test: the EARLIER, already-
    // committed write is untouched by the LATER gate's denial -- not rolled
    // back, corrupted, or masked.
    h.assert_workspace_file_contains("already-in-flight.txt", ALREADY_IN_FLIGHT_CONTENT)
        .await
        .expect(
            "the already-in-flight write from turn 1 must survive turn 2's gate denial \
             unchanged",
        );

    // Non-vacuous: the model's final reply persisted, proving genuine
    // completion rather than a masked/hung terminal state.
    h.assert_reply_contains("was not authorized")
        .await
        .expect("final reply persisted after the denial");
}

/// Pins the CURRENT (undesirable, needs its own follow-up investigation --
/// see the module doc) behavior of the mixed-batch shape this file's main
/// scenario deliberately avoids: a single scripted step with an ungated
/// `builtin.read_file` call before a gated `builtin.write_file` call. Empirically,
/// resuming the write's gate -- even via `approve_gate` -- never re-dispatches
/// it: the file is absent regardless of approve or deny. This test locks in
/// that observation as a named, non-silent regression guard rather than
/// leaving it as an undocumented discovery, so a future fix to the mixed-batch
/// resume path (making `approve_gate` actually create the file) trips this
/// test and prompts an intentional update instead of an unnoticed behavior
/// change.
#[tokio::test]
async fn mixed_batch_gate_resume_does_not_currently_redispatch_the_gated_call() {
    let g = RebornIntegrationGroup::live_approvals()
        .await
        .expect("group builds");
    let h = g
        .thread("conv-mixed-batch-gate-resume-pin")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/seed.txt", "content": ALREADY_IN_FLIGHT_CONTENT}),
            ),
            RebornScriptedReply::text("seed written"),
            RebornScriptedReply::tool_calls([
                ("builtin.read_file", json!({"path": "/workspace/seed.txt"})),
                (
                    "builtin.write_file",
                    json!({"path": "/workspace/mixed-batch.txt", "content": "should have persisted"}),
                ),
            ]),
            RebornScriptedReply::text("read the seed and wrote the second file"),
        ])
        .build()
        .await
        .expect("harness builds");

    let (seed_run, seed_gate) = h
        .submit_turn_until_blocked("seed the file")
        .await
        .expect("blocked on the seed write's approval gate");
    h.approve_gate(seed_run, &seed_gate)
        .await
        .expect("seed gate approved");
    h.wait_for_status(seed_run, TurnStatus::Completed)
        .await
        .expect("seed turn completes");

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("read the seed and write a second file")
        .await
        .expect("blocked on the mixed batch's write gate");
    h.assert_tool_invoked("builtin.read_file")
        .await
        .expect("the ungated read dispatched before the write's gate blocked the batch");

    // Approving (not denying) the gate -- if the write genuinely re-dispatched
    // on resume, "mixed-batch.txt" would exist afterward.
    h.approve_gate(run_id, &gate_ref)
        .await
        .expect("mixed-batch gate approved");
    h.wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("run reaches Completed after the approval");

    h.assert_workspace_file_absent("mixed-batch.txt")
        .await
        .expect(
            "CURRENT (documented, follow-up-needed) behavior: approving a gate raised by the \
             non-first call in a mixed batch does not re-dispatch the write. If this now \
             fails, the mixed-batch resume path was fixed -- update this test (and the \
             module doc) to reflect the corrected behavior instead of just deleting the \
             assertion.",
        );
}
