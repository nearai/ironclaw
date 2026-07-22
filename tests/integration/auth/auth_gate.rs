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
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowKind, AuthFlowOutcome,
    AuthFlowRecord, AuthFlowState, AuthGateRef, AuthProductScope, AuthProviderId, AuthSurface,
    NewAuthFlow, OAuthAuthorizationUrl, OpaqueStateHash, TurnRunRef,
};
use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_product_adapters::{AuthResolutionResult, ProductInboundAck};
use ironclaw_reborn_composition::{
    RebornAuthResolutionDispatcher, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    RebornProductAuthServices,
};
use ironclaw_turns::{GateRef, TurnRunId, TurnStatus};
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

    // `short_circuit_gate_resume` (capabilities.rs ~1149) persists a raw
    // planner summary bypassing the "capability denied with " prefix, so
    // `assert_tool_error(Denied, ..)` can't express this; use the raw-summary
    // assertion instead.
    harness
        .assert_tool_error_summary_contains("auth gate denied by user")
        .await
        .expect("the deny short-circuit's planner summary was persisted");
}

/// User journey: an explicit in-channel `auth deny` is a user abort, not a
/// provider-page denial. It flows through the real product interaction seam,
/// resolves the durable auth flow as `UserAborted`, and cancels only the run
/// parked on the exact gate reference.
#[tokio::test]
async fn explicit_user_abort_cancels_the_exact_blocked_auth_run() {
    let group = RebornIntegrationGroup::builder()
        .with_real_gate_dispatch_services()
        .live_auth_and_approval()
        .await
        .expect("auth-gate group with real interaction services builds");
    let harness = group
        .thread("conv-auth-gate-user-abort")
        .script([RebornScriptedReply::tool_call(
            "github.get_repo",
            serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
        )])
        .build()
        .await
        .expect("thread builds");

    let (run_id, approval_gate_ref) = harness
        .submit_turn_until_blocked("look up a repository")
        .await
        .expect("run first blocks on its capability approval");
    harness
        .approve_gate(run_id, &approval_gate_ref)
        .await
        .expect("approving the action advances the same run");
    let auth_state = harness
        .wait_for_status(run_id, TurnStatus::BlockedAuth)
        .await
        .expect("the approved run reaches its credential gate");
    let gate_ref = auth_state
        .gate_ref
        .expect("blocked-auth state carries the exact gate ref");
    let product_auth = harness
        .product_auth_for_test()
        .expect("group carries its composed product-auth services");
    let flow = create_open_gate_oauth_flow(
        &harness,
        product_auth.as_ref(),
        run_id,
        &gate_ref,
        "github",
        0x3c,
    )
    .await;
    let ack = harness
        .submit_auth_resolution(&gate_ref, AuthResolutionResult::Denied)
        .await
        .expect("the explicit user abort reaches the real auth interaction service");
    assert!(
        matches!(ack, ProductInboundAck::Accepted { .. }),
        "the durable auth-abort input must be acknowledged: {ack:?}"
    );

    harness
        .wait_for_status(run_id, TurnStatus::Cancelled)
        .await
        .expect("the exact blocked run is canceled rather than left parked");
    let durable = product_auth
        .flow_manager()
        .get_flow(&flow.scope, flow.id)
        .await
        .expect("durable flow read succeeds")
        .expect("user-aborted flow remains readable");
    assert_eq!(
        durable.state,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted)
    );
    harness
        .assert_network_egress_count(0)
        .await
        .expect("aborting before dispatch must not invoke the provider");
}

/// User journey: canceling on the provider page is a provider denial. The
/// durable resolution releases only the exact parked gate with a denied
/// disposition; replaying the same terminal event after the gate has moved is
/// an idempotent no-op.
#[tokio::test]
async fn provider_popup_denial_releases_exact_gate_and_duplicate_is_noop() {
    let group = RebornIntegrationGroup::extension_lifecycle_google_oauth_configured()
        .await
        .expect("OAuth-configured extension group builds");
    let harness = group
        .thread("conv-auth-gate-provider-denial")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                serde_json::json!({"extension_id": "google-calendar"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                serde_json::json!({"extension_id": "google-calendar"}),
            ),
            RebornScriptedReply::text("authorization was denied at the provider"),
        ])
        .build()
        .await
        .expect("thread builds");

    let (run_id, gate_ref) = harness
        .submit_turn_until_auth_blocked("install and connect Google Calendar")
        .await
        .expect("run blocks on an auth gate");
    harness
        .wait_for_status(run_id, TurnStatus::BlockedAuth)
        .await
        .expect("the same run remains parked while its popup is open");
    let product_auth = harness
        .product_auth_for_test()
        .expect("group carries its composed product-auth services");
    // The adjacent configured-URL journey proves manifest + administrator
    // configuration -> production AuthEngine URL generation. This journey
    // starts at the popup callback boundary: seed the durable Open flow that
    // the prompt renderer would have created, then drive the real callback,
    // durable resolution dispatch, exact-gate resume, and replay path.
    let flow = create_open_gate_oauth_flow(
        &harness,
        product_auth.as_ref(),
        run_id,
        &gate_ref,
        "google",
        0x4d,
    )
    .await;
    assert!(
        matches!(
            flow.challenge.as_ref(),
            Some(AuthChallenge::OAuthUrl { .. })
        ),
        "the user journey must exercise a real OAuth popup flow: {flow:?}"
    );
    // This integration group executes turns in its shared coordinator. Clone
    // the production callback bundle with the same exact-gate dispatcher the
    // binary composes, aimed at that coordinator; every durable auth port
    // remains the original composed instance.
    let callback_services =
        product_auth
            .as_ref()
            .clone()
            .with_resolution_dispatcher(std::sync::Arc::new(
                ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher::new(
                    harness.turn_coordinator_for_test(),
                ),
            )
                as std::sync::Arc<dyn RebornAuthResolutionDispatcher>);
    let callback_scope = flow.scope.clone();
    let callback_flow_id = flow.id;
    let state_hash = flow
        .opaque_state_hash
        .clone()
        .expect("Open popup flow persists its state hash");
    let denial = callback_services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: callback_scope.clone(),
            flow_id: callback_flow_id,
            opaque_state_hash: state_hash.clone(),
            outcome: RebornOAuthCallbackOutcome::ProviderDenied,
        })
        .await
        .expect_err("provider denial is the route-visible terminal result");
    assert_eq!(denial.code, AuthErrorCode::ProviderDenied);

    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("provider denial completes without re-blocking");
    harness
        .assert_network_egress_count(0)
        .await
        .expect("provider denial before dispatch must not invoke the provider");
    harness
        .assert_tool_error_summary_contains("auth gate denied by user")
        .await
        .expect("the provider denial is visible to the model as a denied gate");

    let durable = product_auth
        .flow_manager()
        .get_flow(&flow.scope, flow.id)
        .await
        .expect("durable flow read succeeds")
        .expect("provider-denied flow remains readable");
    assert_eq!(
        durable.state,
        AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied)
    );
    assert!(
        durable.resolution_delivered_at.is_some(),
        "exact-gate resolution must be durably marked delivered"
    );

    let duplicate = callback_services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: callback_scope,
            flow_id: callback_flow_id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::ProviderDenied,
        })
        .await
        .expect_err("a replay reports the same provider denial");
    assert_eq!(duplicate.code, AuthErrorCode::ProviderDenied);
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("duplicate callback delivery is an idempotent no-op");
}

/// Seed the durable popup-flow precondition around one exact blocked gate.
/// URL generation itself is covered by `oauth_popup_journeys`; auth-gate
/// journeys use this helper to stay focused on resolution and run recovery.
async fn create_open_gate_oauth_flow(
    harness: &RebornIntegrationHarness,
    product_auth: &RebornProductAuthServices,
    run_id: TurnRunId,
    gate_ref: &GateRef,
    provider: &str,
    state_fill: u8,
) -> AuthFlowRecord {
    let owner_user_id = harness
        .turn_scope
        .explicit_owner_user_id()
        .cloned()
        .expect("integration thread has an explicit owner");
    let scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: harness.turn_scope.tenant_id.clone(),
            user_id: owner_user_id,
            agent_id: harness.turn_scope.agent_id.clone(),
            project_id: harness.turn_scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(harness.turn_scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Callback,
    );
    let state_hash =
        OpaqueStateHash::new(format!("{state_fill:02x}").repeat(32)).expect("valid state hash");
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(5);
    product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope,
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new(provider).expect("valid provider"),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://provider.example.test/oauth/authorize",
                )
                .expect("valid authorization URL"),
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).expect("valid turn run ref"),
                gate_ref: AuthGateRef::new(gate_ref.as_str()).expect("valid auth gate ref"),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash),
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("the blocked gate has a durable Open OAuth flow")
}

/// A failed or expired OAuth flow releases the exact auth gate. Auth retains
/// the precise terminal outcome, while the rollback-safe turn resume uses the
/// existing terminal disposition and must not re-dispatch the same
/// missing-credential call into a fresh auth gate.
#[tokio::test]
async fn github_auth_gate_error_resume_completes_without_reblocking() {
    let group = RebornIntegrationGroup::live_auth_gate()
        .await
        .expect("auth-gate group builds");
    let harness = group
        .thread("conv-auth-gate-error")
        .script([
            RebornScriptedReply::tool_call(
                "github.create_issue",
                serde_json::json!({"owner": "o", "repo": "r", "title": "t", "body": "b"}),
            ),
            RebornScriptedReply::text("the authentication flow failed; please reconnect"),
        ])
        .build()
        .await
        .expect("thread builds");

    let (run_id, gate_ref) = harness
        .submit_turn_until_auth_blocked("file an issue")
        .await
        .expect("run blocks on an auth gate");

    harness
        .resume_failed_auth_gate(run_id, &gate_ref)
        .await
        .expect("failed auth flow resumes the exact gate");
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("failed auth resume completes without re-blocking");

    harness
        .assert_no_tool_error(ToolErrorClass::Failed, "backend")
        .await
        .expect("failed auth resume must not re-dispatch the parked capability");
    harness
        .assert_tool_error_summary_contains("auth gate denied by user")
        .await
        .expect("the terminal no-credential outcome is persisted for model recovery");
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
        ironclaw_host_api::VendorId::new("github").expect("valid provider id"),
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
