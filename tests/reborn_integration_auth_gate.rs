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
//! DEFERRED here, COVERED elsewhere: the happy "submit credentials → resume
//! completes" arm. The `live_auth_gate` fixture wires a FIXED `AuthRequired`
//! credential-account resolver with no toggle to flip it to resolved mid-test
//! (and no `run_state` store, so the capability host's auth-resume path cannot
//! complete on this fixture at all). That arm is now covered by
//! `tests/reborn_group_journeys/` (C-JOURNEY) on the
//! `RebornIntegrationGroup::live_auth_and_approval()` group, whose auth gate
//! resolves through the REAL `ProductAuthRuntimeCredentialResolver` +
//! production manual-token flow — no settable-resolver fake needed.
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

use ironclaw_turns::{GateRef, TurnStatus};
use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
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

/// W4-AUTHGATE-WIRE (flagship): a GitHub capability whose credential account
/// resolves OK (`.with_github_issue_tools()` — unlike the sibling test above,
/// whose `live_auth_gate` fixture is wired via `github_issue_tools_auth_required`'s
/// credential-*missing* resolver) but whose injected token draws a runtime
/// `401` from the (scripted) GitHub API must raise `TurnStatus::BlockedAuth`
/// with `credential_requirements` POPULATED (provider=github, ManualToken
/// setup) — the #5174/#5180 bug class: an empty `credential_requirements` left
/// `AuthPromptView.provider` null, so the WebUI's manual-token card threw
/// client-side and silently dropped the submit ("Could not save the token", no
/// network request ever sent).
///
/// This runs the FULL scripted-gateway integration harness (`submit_turn` ->
/// product workflow -> turn coordinator -> agent loop -> capability host -> the
/// real GitHub WASM module), a tier below neither of the two existing pins for
/// this fix covers: `ironclaw_capabilities/tests/capability_host_auth_required_enrichment_contract.rs`
/// drives `CapabilityHost::invoke_json` directly (no turn/loop), and
/// `ironclaw_host_runtime/tests/github_wasm_runtime_contract.rs`
/// (`host_runtime_services_maps_google_drive_wasm_401_to_auth_required`) drives
/// `HostRuntimeServices::invoke_capability` directly (no coordinator/loop, and a
/// different first-party extension). Neither exercises the real submit-turn ->
/// `BlockedAuth` wire the WebUI actually depends on.
#[tokio::test]
async fn runtime_401_after_injection_populates_provider_credential_requirement() {
    let harness = RebornIntegrationHarness::test_default()
        .with_github_network_status(401)
        .script([
            RebornScriptedReply::tool_call(
                "github.get_repo",
                serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text("could not authenticate to github"),
        ])
        .build()
        .await
        .expect("harness builds");

    let (run_id, gate_ref) = harness
        .submit_turn_until_auth_blocked("look up the repo")
        .await
        .expect("run blocks on an auth gate");

    // `submit_turn_until_auth_blocked` only returns the gate ref; re-fetch the
    // full state (already at `BlockedAuth`, so this returns immediately) to
    // read `credential_requirements` — the exact field the #5180 fix enriches
    // from the capability's declared `InjectCredentialAccountOnce` obligation.
    let state = harness
        .wait_for_status(run_id, TurnStatus::BlockedAuth)
        .await
        .expect("run still blocked on the same auth gate");
    assert_eq!(
        state.credential_requirements.len(),
        1,
        "expected exactly one enriched credential requirement -- an empty list \
         here is the provider-null, unsubmittable gate (#5174/#5180 regression); \
         got {:?}",
        state.credential_requirements
    );
    let requirement = &state.credential_requirements[0];
    assert_eq!(
        requirement.provider,
        ironclaw_host_api::RuntimeCredentialAccountProviderId::new("github")
            .expect("valid provider id"),
        "provider must be populated so AuthPromptView.provider is non-null"
    );
    assert_eq!(
        requirement.setup,
        ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
        "expected the ManualToken setup GithubHarnessAuthorizer declares -- a \
         wrong setup kind would route the WebUI to the wrong re-auth UI"
    );

    // Drain the run cleanly (deny) so the test doesn't leak a blocked run.
    harness
        .deny_auth_gate(run_id, &gate_ref)
        .await
        .expect("deny + resume auth gate");
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("denied auth resume completes");
}

/// W4-AUTHGATE-WIRE: cancelling a run parked at `BlockedAuth` must land
/// directly on `TurnStatus::Cancelled` (unlike a mid-model park -- see
/// `reborn_integration_cancel.rs` -- a blocked-gate run has no active worker to
/// cooperate with cancellation, so `request_cancel_once` transitions it
/// straight through) and must leave no stale replay: once cancelled, the SAME
/// real gate ref can no longer resume the run (closes the #5067/#4957 class of
/// auth gates staying "live"/resumable after the user has moved on).
#[tokio::test]
async fn cancel_blocked_auth_gate_leaves_no_stale_replay() {
    let harness = RebornIntegrationHarness::test_default()
        .with_github_network_status(401)
        .script([RebornScriptedReply::tool_call(
            "github.get_repo",
            serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
        )])
        .build()
        .await
        .expect("harness builds");

    let (run_id, gate_ref) = harness
        .submit_turn_until_auth_blocked("look up the repo")
        .await
        .expect("run blocks on an auth gate");
    // Exactly one egress attempt (the 401 probe) before cancel.
    harness
        .assert_network_egress_count(1)
        .await
        .expect("the single 401 probe was recorded before cancel");

    let cancel = harness
        .cancel_run(run_id)
        .await
        .expect("cancel request accepted");
    assert_eq!(
        cancel.status,
        TurnStatus::Cancelled,
        "a BlockedAuth run has no active worker to cooperate with; cancel must \
         land it directly on Cancelled, not CancelRequested"
    );
    harness
        .wait_for_status(run_id, TurnStatus::Cancelled)
        .await
        .expect("run stays Cancelled");

    // Stale-replay guard: resuming the now-terminal run with the SAME real gate
    // ref (not a bogus one -- this proves the run's terminal status is what
    // blocks the resume, not a gate-ref mismatch) must fail, not silently
    // re-dispatch the parked github capability.
    let resume_err = harness
        .deny_auth_gate(run_id, &gate_ref)
        .await
        .expect_err("resuming a cancelled run must fail, not silently replay");
    let resume_err = resume_err.to_string();
    assert!(
        resume_err.contains("invalid turn transition")
            && resume_err.contains("Cancelled")
            && resume_err.contains("Queued"),
        "expected the cancelled/terminal run to be rejected, got: {resume_err}"
    );
    // No further egress escaped the failed resume attempt.
    harness
        .assert_network_egress_count(1)
        .await
        .expect("cancel + failed resume must not trigger a second github dispatch");
}

/// Regression guard for the flip side of the stale-replay test above: an
/// invalid/unknown `GateRef` string against a still-open (non-terminal) run
/// must also fail cleanly rather than resuming under a synthesized ref.
/// Cheap, discriminating companion assertion -- not a full scenario -- so it
/// stays inline here instead of a third full harness build.
#[tokio::test]
async fn deny_auth_gate_rejects_a_non_auth_gate_ref_prefix() {
    let harness = RebornIntegrationHarness::test_default()
        .with_github_network_status(401)
        .script([RebornScriptedReply::tool_call(
            "github.get_repo",
            serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
        )])
        .build()
        .await
        .expect("harness builds");

    let (run_id, _gate_ref) = harness
        .submit_turn_until_auth_blocked("look up the repo")
        .await
        .expect("run blocks on an auth gate");

    let wrong_prefix_ref = GateRef::new("gate:approval-not-an-auth-gate").expect("valid gate ref");
    let result = harness
        .deny_auth_gate(run_id, &wrong_prefix_ref)
        .await
        .expect_err("a non-`gate:auth-` ref must be rejected client-side before it ever reaches the coordinator resume call");
    let result = result.to_string();
    assert!(
        result.contains("auth gate ref")
            || result.contains("gate:auth-")
            || result.contains("invalid gate ref"),
        "expected an invalid auth-gate prefix rejection, got: {result}"
    );
}
