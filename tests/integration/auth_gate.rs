//! E-AUTHGATE: a capability whose credential account resolves `AuthRequired`
//! raises a real `TurnStatus::BlockedAuth` gate; denying it resumes to
//! completion without re-dispatching the parked capability.
//!
//! Path: scripted `github.*` call -> `FixedRuntimeCredentialAccountResolver`
//! returns `AuthRequired` -> `BlockedAuth` (`gate:auth-` ref) -> deny+resume ->
//! the deny short-circuit in `executor/capabilities.rs`
//! (`state.pending_auth_resume` check) surfaces gate-declined instead of
//! re-dispatching. Only the model is faked.
//!
//! DEFERRED here, COVERED by `tests/reborn_group_journeys/` (C-JOURNEY) via
//! `RebornIntegrationGroup::live_auth_and_approval()`: the "submit credentials
//! -> resume completes" arm, since this fixture's resolver is fixed
//! `AuthRequired` with no `run_state` store to complete a real resume.
//!
//! `assert_tool_error` (not just `wait_for_status(Completed)`) is required:
//! mutation-testing the deny short-circuit proved `Completed` alone doesn't
//! discriminate — a bypassed short-circuit still re-dispatches, fails
//! `Backend`, and that failure ALSO finalizes `Completed`. Only the persisted
//! tool-error class/reason distinguishes them.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
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
    // prefix; the `.expect` above is the real failure point.

    harness
        .deny_auth_gate(run_id, &gate_ref)
        .await
        .expect("deny + resume auth gate");

    // Final reply text intentionally NOT asserted: `TraceLlm` emits scripted
    // replies by call order regardless of model behavior, so it wouldn't
    // discriminate a correct deny from a looping regression.
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("denied auth resume completes without re-blocking / looping");

    // Mutation-verified: a `Failed{Backend}` result only exists if the deny
    // short-circuit was bypassed and re-dispatched (see module doc).
    harness
        .assert_no_tool_error(ToolErrorClass::Failed, "backend")
        .await
        .expect(
            "expected no persisted Failed{Backend} tool-error for github.create_issue (a leaked \
             re-dispatch)",
        );

    // `short_circuit_denied_resume` (capabilities.rs ~1149) persists a raw
    // planner summary bypassing the "capability denied with " prefix, so
    // `assert_tool_error(Denied, ..)` can't express this; use the raw-summary
    // assertion instead.
    harness
        .assert_tool_error_summary_contains("auth gate denied by user")
        .await
        .expect("the deny short-circuit's planner summary was persisted");
}

/// W4-AUTHGATE-WIRE (flagship): a GitHub capability with a valid credential
/// account but a runtime `401` from the GitHub API must raise
/// `TurnStatus::BlockedAuth` with `credential_requirements` POPULATED
/// (provider=github, ManualToken) — the #5174/#5180 bug class: an empty
/// `credential_requirements` left `AuthPromptView.provider` null, silently
/// dropping the WebUI manual-token submit ("Could not save the token").
///
/// Runs the full scripted-gateway harness (submit_turn -> workflow ->
/// coordinator -> agent loop -> capability host -> real GitHub WASM module) —
/// a tier neither existing pin covers:
/// `capability_host_auth_required_enrichment_contract.rs` drives
/// `CapabilityHost::invoke_json` directly (no turn/loop);
/// `github_wasm_runtime_contract.rs` drives `HostRuntimeServices::invoke_capability`
/// directly (no coordinator/loop). Neither exercises the real submit-turn ->
/// `BlockedAuth` wire the WebUI depends on.
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

    // Re-fetch full state to read `credential_requirements` -- the field the
    // #5180 fix enriches from the InjectCredentialAccountOnce obligation.
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

    // T2 of the #6105 lifecycle transitions — the #5878 negative
    // discriminator: a revoked/rejected token (provider 401) must surface
    // EXCLUSIVELY as the re-auth gate asserted above, never as a generic
    // model-visible failure ("the tool input could not be encoded" / "AI
    // model provider was temporarily unavailable"). An empty reason matches
    // any summary of the class, so this pins that NO Failed-classed tool
    // error of any reason was persisted for the 401.
    harness
        .assert_no_tool_error(ToolErrorClass::Failed, "")
        .await
        .expect(
            "a provider 401 must park at the auth gate, not persist a generic \
             Failed tool error (#5878 misleading-error regression)",
        );
    // ...and it must not hot-retry the rejected credential: exactly the one
    // 401 probe crossed the wire (#5878's "multiple retry attempts" shape).
    harness
        .assert_network_egress_count(1)
        .await
        .expect("a 401 must not be hot-retried against the provider");

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

/// W4-AUTHGATE-WIRE: cancelling a run parked at `BlockedAuth` lands directly
/// on `Cancelled` (no active worker to cooperate with, unlike a mid-model
/// park) and leaves no stale replay -- the SAME gate ref can't resume after
/// cancel (closes the #5067/#4957 "gate stays live" class).
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

    // Stale-replay guard: the SAME gate ref (not bogus) must be rejected by
    // the run's terminal status, not a gate-ref mismatch.
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

/// Regression guard, flip side of the stale-replay test above: an
/// invalid/unknown `GateRef` against a still-open run must also fail cleanly,
/// not resume under a synthesized ref.
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
