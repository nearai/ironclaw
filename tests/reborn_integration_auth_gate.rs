//! E-AUTHGATE seam test: a capability whose credential account resolves to
//! `AuthRequired` raises a real `TurnStatus::BlockedAuth` gate, and denying
//! that gate resumes the run to completion without re-dispatching the parked
//! capability (no loop, no silent re-execution).
//!
//! Drives the production auth path end-to-end: scripted `github.*` tool call →
//! real credential-account injection (`FixedRuntimeCredentialAccountResolver`
//! returns `AuthRequired`) → `CapabilityObligationError::AuthRequired` → the
//! agent loop blocks the run at `BlockedAuth` with a `gate:auth-` ref → deny +
//! resume → the executor's deny short-circuit
//! (`crates/ironclaw_agent_loop/src/executor/capabilities.rs`, the
//! `state.pending_auth_resume` disposition check right after the
//! `visible_calls.is_empty()` guard) surfaces a model-visible gate-declined
//! failure for the parked capability instead of re-dispatching it → the run
//! completes. Nothing is faked except the model at the vendor-SDK seam.
//!
//! DEFERRED: the happy "submit credentials → resume completes" arm. The
//! `live_auth_gate` fixture wires a FIXED `AuthRequired` credential-account
//! resolver with no toggle to flip it to resolved mid-test; exercising that
//! arm needs a new settable-resolver seam, which is out of scope for this
//! pure-coverage PR.
//!
//! `assert_tool_error` IS used below, despite the general guidance to prefer
//! `wait_for_status(Completed)` as the sole discriminator (as in
//! `tests/reborn_group_approvals/scenario_gate_then_deny.rs`): mutation-testing
//! this specific short-circuit (deleting the disposition check) showed that
//! `wait_for_status(Completed)` alone does NOT fail. This harness's mock
//! capability host has no support for a genuine auth-resume completion (see
//! the DEFERRED note above), so a neutralized short-circuit still reaches
//! `Completed` — just via a *different*, harness-specific path: the
//! re-dispatched call comes back `Failed(Backend, "... resume requires
//! run_state")` instead of being denied, and that Failed observation is ALSO
//! surfaced to the model as a non-blocking failure, which also finalizes to
//! `Completed`. `wait_for_status(Completed)` cannot tell these apart; the
//! persisted tool-error class/reason can, because a real re-dispatch is the
//! only way a `Failed{Backend}` result is ever recorded for this capability in
//! this test.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use ironclaw_turns::TurnStatus;
use reborn_support::assertions::ToolErrorClass;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn github_auth_gate_denied_resume_completes_without_loop() {
    let group = RebornIntegrationGroup::live_auth_gate()
        .await
        .expect("auth-gate group builds");
    let harness = group
        .thread("conv-auth-gate")
        .script([
            RebornScriptedReply::tool_call(
                "github.create_issue",
                serde_json::json!({"owner": "o", "repo": "r", "title": "t", "body": "b"}),
            ),
            // Consumed by the model call after the gate-declined observation;
            // its content is intentionally NOT asserted (see below).
            RebornScriptedReply::text("could not file the issue"),
        ])
        .build()
        .await
        .expect("thread builds");

    let (run_id, gate_ref) = harness
        .submit_turn_until_auth_blocked("file an issue")
        .await
        .expect("run blocks on an auth gate");
    // `submit_turn_until_auth_blocked` already validates the `gate:auth-`
    // prefix and returns `Err` otherwise, so the `.expect` above is the real
    // failure point — no redundant assert needed here.

    harness
        .deny_auth_gate(run_id, &gate_ref)
        .await
        .expect("deny + resume auth gate");

    // The scripted final reply text is intentionally NOT asserted: `TraceLlm`
    // emits scripted replies by call order regardless of what the model
    // actually observed, so asserting its text would not distinguish a correct
    // deny-and-continue from a regression that loops or fails differently
    // (same reasoning as
    // tests/reborn_group_approvals/scenario_gate_then_deny.rs). Do not call
    // `assert_reply_contains` here.
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("denied auth resume completes without re-blocking / looping");

    // The discriminating proof: no `Failed{Backend}` tool-error was persisted
    // for this capability. Mutation-verified — see the module doc above — a
    // `Failed{Backend, "resume requires run_state"}` result only exists when
    // the deny short-circuit was bypassed and re-dispatched.
    harness
        .assert_no_tool_error(ToolErrorClass::Failed, "backend")
        .await
        .expect(
            "expected no persisted Failed{Backend} tool-error for github.create_issue (a leaked \
             re-dispatch)",
        );

    // Positive proof of the CORRECT outcome: `short_circuit_denied_resume`
    // (capabilities.rs ~1149) persists its raw planner summary via
    // `SanitizedStrategySummary::from_trusted_static("auth gate denied by
    // user")`, deliberately bypassing the "capability denied with " prefix
    // (no host-returned text to prefix for a gate denial) — so
    // `assert_tool_error(Denied, ..)` cannot express this; use the raw-summary
    // assertion instead.
    harness
        .assert_tool_error_summary_contains("auth gate denied by user")
        .await
        .expect("the deny short-circuit's planner summary was persisted");
}
