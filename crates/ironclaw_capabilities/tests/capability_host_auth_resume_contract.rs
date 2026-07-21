// arch-exempt: large_file, mechanical lease-store test repoint to FilesystemCapabilityLeaseStore<InMemoryBackend> helper (arch-simplification §4.3), no new test logic, plan #6168
// Contract tests for `CapabilityHost::auth_resume_json`.
//
// Covers: lease survival across auth-gate re-dispatch, concurrent-claim race
// handling, terminal vs non-terminal bounce lease disposition, approval
// fingerprint validation, and capability-existence check ordering vs lease
// acquisition (unknown-capability must not strand leases).
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::*;
use ironclaw_run_state::*;
use serde_json::json;

mod support;
use support::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dispatch_lease_approval() -> LeaseApproval {
    LeaseApproval {
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: vec![],
            resource_ceiling: None,
            expires_at: None,
            max_invocations: Some(1),
        },
    }
}

// ---------------------------------------------------------------------------
// Deliverable 1a: accepts run in BlockedAuth and dispatches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_accepts_blocked_auth_run_and_dispatches() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    // Simulate a run that was previously blocked at an auth gate.
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs credential"});

    // Manually start and block the run at auth so auth_resume_json can act on it.
    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let result = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input,
            approval_request_id: None,
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.call_count() > 0);
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}

#[tokio::test]
async fn auth_resume_preserves_original_actor_and_rejects_forged_actor() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    let mut alice_context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let alice = UserId::new("slack-alice").expect("valid authenticated actor user id");
    alice_context.authenticated_actor_user_id = Some(alice.clone());
    let scope = alice_context.resource_scope.clone();
    let invocation_id = alice_context.invocation_id;
    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: Some(alice),
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let mut forged_context = alice_context.clone();
    forged_context.authenticated_actor_user_id =
        Some(UserId::new("slack-bob").expect("valid forged authenticated actor user id"));
    let forged_error = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: forged_context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "resume"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            forged_error,
            CapabilityInvocationError::AuthorizationDenied {
                reason: DenyReason::PolicyDenied,
                ..
            }
        ),
        "changed authenticated actor must fail closed, got {forged_error:?}"
    );
    assert_eq!(dispatcher.call_count(), 0);

    host.auth_resume_json(CapabilityAuthResumeRequest {
        context: alice_context,
        capability_id: capability_id(),
        estimate: ResourceEstimate::default(),
        input: json!({"message": "resume"}),
        approval_request_id: None,
    })
    .await
    .expect("the original authenticated actor can resume");

    let dispatched = dispatcher.last_request().unwrap();
    assert_eq!(
        dispatched
            .authenticated_actor_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("slack-alice")
    );
}

// ---------------------------------------------------------------------------
// Deliverable 1b: rejects run NOT in BlockedAuth
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_run_in_blocked_approval_status() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();

    // Block the invocation at an approval gate (not auth).
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
        })
        .await
        .unwrap_err();

    // Run is now BlockedApproval — auth_resume_json must reject it.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);

    let auth_authorizer = GrantAuthorizer::new();
    let auth_host =
        capability_host(&registry, &dispatcher, &auth_authorizer).with_run_state(&run_state);
    let err = auth_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ResumeNotBlocked {
                status: RunStatus::BlockedApproval,
                ..
            }
        ),
        "expected ResumeNotBlocked(BlockedApproval), got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "auth_resume must not dispatch when run is not in BlockedAuth"
    );
}

#[tokio::test]
async fn auth_resume_json_rejects_run_in_running_status() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    // Only start the run (Running status), do NOT block_auth.
    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "try to resume running"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ResumeNotBlocked {
                status: RunStatus::Running,
                ..
            }
        ),
        "expected ResumeNotBlocked(Running), got {err:?}"
    );
    assert!(dispatcher.call_count() == 0);
}

// ---------------------------------------------------------------------------
// Deliverable 1c: rejects invocation-fingerprint mismatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_fingerprint_mismatch_on_approval_request() {
    // Build an invocation, approve it (fingerprinted to original input), then
    // attempt auth_resume with different input — should reject before lease claim.
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Phase 1: invoke (needs approval).
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let original_input = json!({"message": "original approved input"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: original_input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // Approve (issues a fingerprinted lease for the original input).
    ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();

    // Phase 2: dispatch triggers auth (simulate by manually moving to BlockedAuth).
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Phase 3: auth_resume with MUTATED input — fingerprint will not match lease.
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: original_context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "MUTATED input"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
        ),
        "mutated input must be rejected as fingerprint mismatch, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when fingerprint mismatches"
    );
    // Lease must still be active (not consumed or revoked).
    // (We cannot easily check the lease status here without knowing the lease id,
    // but the lack of dispatch proves no claim happened.)
}

// ---------------------------------------------------------------------------
// Deliverable 1d (clean-ordering shortcut path): with approval_request_id and
// an Active lease — claims it and dispatches successfully.
//
// This is the "fast path" where the approval bounce did NOT go through the
// auth bounce first (i.e., BlockedApproval → BlockedAuth via direct shortcut).
// The real-path test `auth_resume_after_real_approval_bounce_reuses_claimed_lease`
// below covers the case where resume_json ran first and left the lease Claimed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_with_approval_request_id_claims_active_lease_and_dispatches() {
    // Clean-ordering path: approve → manual block_auth (skipping resume_json) →
    // auth_resume_json → finds Active lease, claims it, dispatches.
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Phase 1: first invocation triggers approval.
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let original_invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs both approval and auth"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, original_invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // Phase 2: approve — fingerprinted lease issued for original_invocation_id's scope.
    let _lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();

    // Phase 3: simulate dispatch having returned AuthRequired by moving the run
    // directly to BlockedAuth (shortcut — no resume_json ran, lease is Active).
    run_state
        .block_auth(&scope, original_invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Phase 4: auth_resume_json with the SAME context (preserving correlation_id)
    // and the approval_request_id. Lease is Active → claim → dispatch succeeds.
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer_2 = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer_2)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let result = resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate,
            input,
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap();

    assert_eq!(
        result.dispatch.output,
        json!({"ok": true}),
        "auth_resume with original invocation_id must complete dispatch"
    );
    assert!(dispatcher.call_count() > 0);
    // Run must be completed.
    let run = run_state
        .get(&scope, original_invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    // Approval must still be Approved (consumed via lease path, not re-pending).
    let approval = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        approval.status,
        ApprovalStatus::Approved,
        "approval must remain in Approved state after successful auth_resume"
    );
}

// ---------------------------------------------------------------------------
// auth_resume_json rejects capability_id mismatch against run record
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_capability_id_mismatch_against_run_record() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    // Start the run with the canonical capability_id.
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(), // echo.say
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    // Attempt auth_resume with a DIFFERENT but KNOWN-and-plannable capability_id,
    // so the run-state capability-mismatch check fires. (An unknown id would now
    // short-circuit to `UnknownCapability` in `resume_preflight` before the
    // mismatch check — existence-first, matching host_runtime's deleted
    // pre-authorization; the unknown-capability precedence is covered separately.)
    let different_id = other_capability_id();
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: different_id,
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "mismatch"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(err, CapabilityInvocationError::ResumeContextMismatch { .. }),
        "capability_id mismatch against run record must be rejected, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire on capability_id mismatch"
    );
    // Run must still be in BlockedAuth (not failed) — the run state is preserved
    // except when fail_run_if_configured transitions it.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_ne!(
        run.status,
        RunStatus::Completed,
        "run must not be completed after a mismatch rejection"
    );
}

// ---------------------------------------------------------------------------
// auth_resume_json_rejects_approval_not_yet_approved
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_approval_not_yet_approved() {
    // When approval_request_id is Some but the approval is still Pending,
    // auth_resume_json must return Err(ApprovalNotApproved), fire zero dispatches,
    // and leave the run in its original BlockedAuth status.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Start and block at auth.
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Insert a Pending approval into the store (not yet approved).
    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: context.correlation_id,
                requested_by: Principal::HostRuntime,
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: ResourceEstimate::default(),
                }),
                invocation_fingerprint: None,
                reason: "pending approval".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "pending approval"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ApprovalNotApproved {
                status: ApprovalStatus::Pending,
                ..
            }
        ),
        "expected ApprovalNotApproved(Pending), got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when approval is still Pending"
    );
    // Run must remain in BlockedAuth (Pending approval → no fail_run_if_configured call).
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "run must remain BlockedAuth when approval is Pending"
    );
}

// ---------------------------------------------------------------------------
// auth_resume_json returns ResumeStoreMissing when approval_requests
//         store is absent but approval_request_id is Some.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_returns_store_missing_when_approval_requests_absent() {
    // When auth_resume_json is called with approval_request_id = Some but the
    // host was wired WITHOUT an approval_requests store, the function must return
    // Err(ResumeStoreMissing { store: "approval_requests" }) immediately.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Host has run_state but NO approval_requests store.
    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let approval_id = ApprovalRequestId::new();
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "needs approval store"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ResumeStoreMissing { store, .. }
            if store == "approval_requests"
        ),
        "expected ResumeStoreMissing {{ store: \"approval_requests\" }}, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when approval_requests store is absent"
    );
}

// ---------------------------------------------------------------------------
// Deliverable 1e: without approval_request_id — skips lease path cleanly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_without_approval_request_id_skips_lease_path_and_dispatches() {
    // A capability that only needs auth (no prior approval gate).
    // auth_resume_json with approval_request_id = None must skip lease
    // validation entirely and proceed directly to dispatch.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "auth only, no approval"});

    // Start and block at auth.
    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let result = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input,
            approval_request_id: None, // no prior approval
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.call_count() > 0);
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}

// ---------------------------------------------------------------------------
// REAL-PATH BUG: lease is Revoked after resume_json dispatch auth bounce
//
// This is the regression test for the real approval→auth bounce ordering:
//   invoke → BlockedApproval (approval Pending)
//   → approve (lease issued, Active)
//   → resume_json → dispatcher returns AuthRequired → run BlockedAuth
//       AND lease is revoked at the dispatch-error revoke site
//   → auth_resume_json with approval_request_id=Some
//       → matching_approval_lease (Active only) → None → ApprovalLeaseMissing ← BUG
//
// Post-fix expected behavior:
//   (a) after resume_json bounce: lease status is Claimed, NOT Revoked
//   (b) auth_resume_json succeeds and dispatches (reuses the Claimed lease)
//   (c) approval request is still Approved
//   (d) after success the lease is Consumed
//   (e) capability dispatched with the SAME invocation id as the original
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_after_real_approval_bounce_reuses_claimed_lease() {
    // A dispatcher that returns AuthRequired on the first call and succeeds on
    // the second, so we can drive the real resume_json → auth bounce → auth_resume_json flow.
    let registry = registry_with_echo_capability();
    let dispatcher = TestDispatcher::responding(|request, call_index| {
        if call_index == 0 {
            // First dispatch: return AuthRequired to trigger the auth bounce.
            Err(DispatchError::AuthRequired {
                capability: request.invocation.capability.clone(),
                required_secrets: vec![],
                credential_requirements: vec![],
            })
        } else {
            // Second dispatch (from auth_resume_json): succeed.
            Ok(ok_dispatch_result(request))
        }
    });
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // ── Phase 1: invoke_json → BlockedApproval ──────────────────────────────
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let original_invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs approval then auth"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let run = run_state
        .get(&scope, original_invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedApproval,
        "Phase 1: run must be BlockedApproval after invoke_json"
    );
    let approval_id = run.approval_request_id.unwrap();

    // ── Phase 2: approve → lease issued (Active) ────────────────────────────
    let issued_lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();
    let lease_id = issued_lease.grant.id;

    let lease = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease.status,
        CapabilityLeaseStatus::Active,
        "Phase 2: lease must be Active after approval"
    );
    let approval = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        approval.status,
        ApprovalStatus::Approved,
        "Phase 2: approval must be Approved"
    );

    // ── Phase 3: resume_json → dispatcher returns AuthRequired ──────────────
    // resume_json calls: find lease (Active) → claim (Active→Claimed) → dispatch
    // → DispatchError::AuthRequired → apply_run_state_transition (BlockAuth) →
    // A non-terminal auth bounce must NOT revoke: the lease stays Claimed so the
    // subsequent auth_resume can reuse it (revoking here was the original bug).
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let resume_err = resume_host
        .resume_json(CapabilityResumeRequest {
            context: resume_context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            resume_err,
            CapabilityInvocationError::AuthorizationRequiresAuth { .. }
        ),
        "Phase 3: resume_json must return AuthorizationRequiresAuth, got {resume_err:?}"
    );

    // Verify run transitioned to BlockedAuth.
    let run_after_resume = run_state
        .get(&scope, original_invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run_after_resume.status,
        RunStatus::BlockedAuth,
        "Phase 3: run must be BlockedAuth after resume_json auth bounce"
    );

    // ── Assertion (a): lease must be Claimed, NOT Revoked ───────────────────
    let lease_after_bounce = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after_bounce.status,
        CapabilityLeaseStatus::Claimed,
        "(a) lease must be Claimed after auth bounce, not Revoked — this is the bug pre-fix"
    );

    // ── Phase 4: auth_resume_json → reuses Claimed lease → dispatches ───────
    let auth_resume_authorizer = GrantAuthorizer::new();
    let auth_resume_host = capability_host(&registry, &dispatcher, &auth_resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    // ── Assertion (b): auth_resume_json succeeds ─────────────────────────────
    let auth_result = auth_resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_or_else(|e| {
            panic!(
                "(b) auth_resume_json must succeed after auth bounce, got err: {e:?}\n\
                 This is ApprovalLeaseMissing pre-fix because the lease was Revoked."
            )
        });

    assert_eq!(
        auth_result.dispatch.output,
        json!({"ok": true}),
        "(b) auth_resume_json must dispatch successfully"
    );

    // ── Assertion (c): approval still Approved ───────────────────────────────
    let approval_after = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        approval_after.status,
        ApprovalStatus::Approved,
        "(c) approval must remain Approved after auth_resume_json success"
    );

    // ── Assertion (d): lease is now Consumed ─────────────────────────────────
    let lease_after_success = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after_success.status,
        CapabilityLeaseStatus::Consumed,
        "(d) lease must be Consumed after successful auth_resume_json dispatch"
    );

    // ── Assertion (e): dispatch was called with the SAME invocation_id ────────
    // The second (successful) dispatch is the most recently recorded request.
    let dispatched_request = dispatcher.last_request().unwrap();
    assert_eq!(
        dispatched_request.invocation.scope.invocation_id, original_invocation_id,
        "(e) capability must be dispatched with the original invocation_id"
    );
}

// ---------------------------------------------------------------------------
// Terminal dispatch failure in auth_resume_json revokes the lease
//
// When auth_resume_json encounters a terminal dispatch failure (any error
// other than AuthorizationRequiresAuth, which is the non-terminal BlockAuth
// path), the claimed approval lease must be Revoked — not left Claimed.
// Before the fix the lease was left Claimed because auth_resume_json was
// missing the guarded-revoke logic that resume_json has.
//
// This drives the real path: invoke → approve → resume_json (auth bounce,
// lease stays Claimed) → auth_resume_json with terminal dispatcher → Revoked.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_terminal_dispatch_failure_revokes_claimed_lease() {
    // Phase 1+2 use an auth-bounce dispatcher (AuthRequired on first call
    // so resume_json bounces and leaves the lease Claimed).
    // Phase 3 (auth_resume_json) uses a terminal-fail dispatcher
    // (UnknownCapability on first call so auth_resume_json errors terminally).
    let registry = registry_with_echo_capability();
    let dispatcher = TestDispatcher::scripted(vec![
        // First call (from resume_json): auth bounce → lease stays Claimed.
        Err(DispatchError::AuthRequired {
            capability: capability_id(),
            required_secrets: vec![],
            credential_requirements: vec![],
        }),
        // Second call (from auth_resume_json): terminal failure.
        Err(DispatchError::UnknownCapability {
            capability: capability_id(),
        }),
    ]);
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Phase 1: invoke → BlockedApproval.
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "terminal fail test"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // Phase 2: approve → lease issued (Active).
    let issued_lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();
    let lease_id = issued_lease.grant.id;

    // Phase 3: resume_json — first call returns AuthRequired → lease Claimed
    // (via the non-terminal BlockAuth guard that already exists in resume_json).
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let _ = resume_host
        .resume_json(CapabilityResumeRequest {
            context: resume_context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    // Verify lease is now Claimed (non-terminal auth bounce guard works).
    let lease_after_resume = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after_resume.status,
        CapabilityLeaseStatus::Claimed,
        "pre-condition: lease must be Claimed after resume_json auth bounce"
    );

    // Phase 4: auth_resume_json — second call is terminal (UnknownCapability).
    let auth_resume_authorizer = GrantAuthorizer::new();
    let auth_resume_host = capability_host(&registry, &dispatcher, &auth_resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = auth_resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(err, CapabilityInvocationError::Dispatch { .. }),
        "expected Dispatch error from terminal failure, got {err:?}"
    );

    // Lease must be Revoked after terminal dispatch failure (pre-fix: stayed Claimed).
    let lease_after = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after.status,
        CapabilityLeaseStatus::Revoked,
        "lease must be Revoked after terminal dispatch failure in auth_resume_json \
         (pre-fix: was left Claimed because guarded-revoke was missing)"
    );
}

// ---------------------------------------------------------------------------
// Non-terminal auth bounce in auth_resume_json leaves lease Claimed
//
// When auth_resume_json encounters AuthorizationRequiresAuth (which transitions
// the run to BlockedAuth — the non-terminal path), the claimed approval lease
// must stay Claimed so the NEXT auth_resume_json call can reuse it.
// This is the guard that prevents burning the approval on every auth retry.
//
// This drives: invoke → approve → resume_json (bounce 1, Claimed) →
// auth_resume_json (bounce 2, AuthRequired again) → lease still Claimed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_non_terminal_auth_bounce_leaves_lease_claimed() {
    // A dispatcher that always returns AuthRequired (non-terminal BlockAuth path).
    let registry = registry_with_echo_capability();
    let dispatcher = TestDispatcher::auth_required();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Phase 1: invoke → BlockedApproval.
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "non-terminal auth bounce"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // Phase 2: approve → lease issued (Active).
    let issued_lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();
    let lease_id = issued_lease.grant.id;

    // Phase 3: resume_json → auth bounce → lease Claimed (existing non-terminal guard).
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let _ = resume_host
        .resume_json(CapabilityResumeRequest {
            context: resume_context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    // Verify lease is Claimed before auth_resume_json.
    let lease_after_resume = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after_resume.status,
        CapabilityLeaseStatus::Claimed,
        "pre-condition: lease must be Claimed after resume_json auth bounce"
    );

    // Phase 4: auth_resume_json — dispatcher again returns AuthRequired.
    // This exercises the same reuse path from the prior test but now the
    // dispatcher bounces again: the lease must stay Claimed for another retry.
    let auth_resume_authorizer = GrantAuthorizer::new();
    let auth_resume_host = capability_host(&registry, &dispatcher, &auth_resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = auth_resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::AuthorizationRequiresAuth { .. }
        ),
        "expected AuthorizationRequiresAuth (non-terminal bounce), got {err:?}"
    );

    // Lease must remain Claimed — NOT Revoked — so the next auth_resume can reuse it.
    let lease_after = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after.status,
        CapabilityLeaseStatus::Claimed,
        "lease must remain Claimed after non-terminal BlockAuth bounce in auth_resume_json \
         (pre-fix: guarded-revoke was missing so behavior was undefined)"
    );
    // Verify invocation_id is unchanged (still the original).
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "run must still be BlockedAuth after non-terminal bounce"
    );
}

// ---------------------------------------------------------------------------
// Concurrent auth-resume: lease claim race loser returns lease error without
// failing the run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_auth_resume_claim_loser_returns_lease_error_without_failing_run() {
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    // A lease store that delegates everything to an inner FilesystemCapabilityLeaseStore<InMemoryBackend>
    // except `claim()`, which returns InactiveLease { status: Claimed } once the
    // `fail_next_claim` flag is set — simulating the loser of a concurrent claim race.
    // domain-state fake, not an I/O fault — cannot move to
    // ironclaw_filesystem::FaultInjecting: it returns a domain
    // `CapabilityLeaseError::InactiveLease{Claimed}` (the concurrent winner already
    // claimed) that no backend fault (which only yields `Persistence`) can produce.
    struct ClaimFailingLeaseStore {
        inner: FilesystemCapabilityLeaseStore<InMemoryBackend>,
        fail_next_claim: AtomicBool,
    }

    impl ClaimFailingLeaseStore {
        fn new() -> Self {
            Self {
                inner: in_memory_backed_capability_lease_store(),
                fail_next_claim: AtomicBool::new(false),
            }
        }

        fn arm_claim_failure(&self) {
            self.fail_next_claim.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl CapabilityLeaseStore for ClaimFailingLeaseStore {
        async fn issue(
            &self,
            lease: CapabilityLease,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.issue(lease).await
        }

        async fn revoke(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.revoke(scope, lease_id).await
        }

        async fn get(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Option<CapabilityLease> {
            self.inner.get(scope, lease_id).await
        }

        async fn claim(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            if self.fail_next_claim.swap(false, Ordering::SeqCst) {
                // Return the error a concurrent winner would trigger: the lease
                // is now Claimed (the other caller got there first).
                return Err(CapabilityLeaseError::InactiveLease {
                    lease_id,
                    status: CapabilityLeaseStatus::Claimed,
                });
            }
            self.inner
                .claim(scope, lease_id, invocation_fingerprint)
                .await
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.consume(scope, lease_id).await
        }

        async fn begin_dispatch_claimed(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner
                .begin_dispatch_claimed(scope, lease_id, invocation_fingerprint)
                .await
        }

        async fn abort_dispatch_claimed(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.abort_dispatch_claimed(scope, lease_id).await
        }

        async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
            self.inner.leases_for_scope(scope).await
        }

        async fn active_leases_for_context(
            &self,
            context: &ExecutionContext,
        ) -> Vec<CapabilityLease> {
            self.inner.active_leases_for_context(context).await
        }
    }

    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = ClaimFailingLeaseStore::new();

    // Phase 1: invoke → BlockedApproval.
    let block_host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "concurrent claim race"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // Phase 2: approve → Active lease issued (one-shot).
    ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();

    // Phase 3: move run to BlockedAuth (shortcut, as in the clean-ordering test).
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Phase 4: arm the claim failure to simulate concurrent race loser, then
    // call auth_resume_json — it must return Err(Lease) WITHOUT failing the run.
    leases.arm_claim_failure();

    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let authorizer = GrantAuthorizer::new();
    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(err, CapabilityInvocationError::Lease(_)),
        "concurrent claim loser must return Lease error, got {err:?}"
    );

    // Run must still be BlockedAuth — concurrent-resume loser must not fail the run.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "concurrent claim loser must not transition run to Failed \
         (run left resumable for the winner or a subsequent retry)"
    );

    // Dispatch must not have been called (claim failed before dispatch).
    assert!(
        dispatcher.call_count() == 0,
        "concurrent claim loser must not dispatch"
    );
}

// ---------------------------------------------------------------------------
// TEST 1: ResumeStoreMissing when capability_leases is absent
//
// When `auth_resume_json` is called with `approval_request_id = Some` AND
// the host has `approval_requests` wired BUT no `capability_leases` store,
// the function must return `Err(ResumeStoreMissing { store: "capability_leases" })`
// before any dispatch or run-state transition.
//
// This covers the branch at the second `ok_or_else` inside the
// `if let Some(approval_request_id) = request.approval_request_id` block
// that was not exercised by the existing `auth_resume_json_returns_store_missing_when_approval_requests_absent`
// test (which only covers the missing `approval_requests` branch).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_returns_store_missing_when_capability_leases_absent() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();

    // Start and block at auth.
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Host has approval_requests configured BUT NO capability_leases.
    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    // No .with_capability_leases() call — capability_leases is absent.

    let approval_id = ApprovalRequestId::new();
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "needs lease store"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ResumeStoreMissing { store, .. }
            if store == "capability_leases"
        ),
        "expected ResumeStoreMissing {{ store: \"capability_leases\" }}, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when capability_leases store is absent"
    );
    // Run must remain in BlockedAuth — the missing-store branch must not fail the run.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "run must remain BlockedAuth when capability_leases store is absent"
    );
}

// ---------------------------------------------------------------------------
// TEST 2: Denied (non-Pending) prior approval → fail_run_if_configured +
//         ApprovalNotApproved returned
//
// When `auth_resume_json` is called with `approval_request_id = Some` and
// the referenced approval has a NON-Pending status (e.g. Denied), the
// function must:
//   (a) call `fail_run_if_configured` to transition the BlockedAuth run
//       to Failed (unlike the Pending branch which leaves the run unchanged)
//   (b) return `Err(ApprovalNotApproved { status: Denied })`
//
// This covers the `if approval.status != ApprovalStatus::Pending { ... }`
// branch inside the `!= Approved` early-return path of `auth_resume_json`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejected_prior_approval_fails_blocked_auth_run() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Start and block at auth.
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Insert a Pending approval, then deny it so its status becomes Denied.
    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: context.correlation_id,
                requested_by: Principal::HostRuntime,
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: ResourceEstimate::default(),
                }),
                invocation_fingerprint: None,
                reason: "denied approval".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    // Transition the approval to Denied (non-Pending, non-Approved).
    approval_requests.deny(&scope, approval_id).await.unwrap();

    // Verify precondition: approval is now Denied.
    let pre = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pre.status,
        ApprovalStatus::Denied,
        "precondition: approval must be Denied"
    );

    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "denied approval"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    // Must return ApprovalNotApproved with the actual Denied status.
    assert!(
        matches!(
            err,
            CapabilityInvocationError::ApprovalNotApproved {
                status: ApprovalStatus::Denied,
                ..
            }
        ),
        "expected ApprovalNotApproved(Denied), got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when prior approval is Denied"
    );
    // Unlike the Pending branch (which leaves the run in BlockedAuth),
    // the non-Pending branch calls fail_run_if_configured → run must be Failed.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::Failed,
        "run must be transitioned to Failed when prior approval is Denied \
         (fail_run_if_configured is called for non-Pending rejections)"
    );
}

// ---------------------------------------------------------------------------
// Concurrent auth-resume: begin_dispatch_claimed race loser returns lease
// error without failing the run (Claimed-lease reuse path)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// TEST 4: Rewrite — real winner/loser race for begin_dispatch_claimed
//
// Drive the actual concurrent begin_dispatch_claimed race:
//   1. Setup: invoke → approve → resume_json (auth bounce) → lease is Claimed.
//   2. Two concurrent `auth_resume_json` calls share the same Claimed lease,
//      coordinated by a barrier inside `leases_for_scope` so both callers
//      complete the helper scan before either calls `begin_dispatch_claimed`.
//   3. One caller wins `begin_dispatch_claimed` (Claimed→Dispatching) and
//      then BLOCKS inside a gating dispatcher so the Dispatching state is
//      held while the loser's `begin_dispatch_claimed` is still pending.
//   4. The loser's `begin_dispatch_claimed` sees `Dispatching` and returns
//      `InactiveLease { status: Dispatching }` — asserted on exact variant
//      and status (not just `Lease(_)`).
//   5. The loser does NOT fail the run (run stays BlockedAuth).
//   6. The winner is released; exactly ONE dispatch completes.
//
// Synchronization: a `tokio::sync::Barrier(2)` is embedded in the lease
// store's `leases_for_scope` so both concurrent callers rendezvous after
// completing the helper scan but before either calls `begin_dispatch_claimed`.
// A `Notify` pair gates the winner's dispatcher so the loser can be
// observed while the winner holds the Dispatching state.  No sleeps.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_auth_resume_reuse_loser_does_not_double_dispatch() {
    use async_trait::async_trait;
    use std::sync::Arc as StdArc;
    use tokio::sync::{Barrier, Notify};

    // ── Barrier lease store ──────────────────────────────────────────────────
    // Wraps FilesystemCapabilityLeaseStore<InMemoryBackend> and inserts a Barrier(2) inside
    // `leases_for_scope`.  Both concurrent callers block at the barrier
    // inside the helper scan, then both are released together.  This
    // guarantees both see the Claimed lease before either calls
    // `begin_dispatch_claimed`, creating the real data race on the state
    // machine transition: one wins (Claimed→Dispatching), the other loses
    // (sees Dispatching → InactiveLease{Dispatching}).
    // Synchronization primitive, not an I/O fault — cannot move to
    // ironclaw_filesystem::FaultInjecting: FaultInjecting injects errors + records
    // ops but is explicitly NOT a read/write-interleaving barrier (see
    // ironclaw_filesystem/CLAUDE.md). The Barrier(2) interleave is the whole test.
    struct BarrierLeaseStore {
        inner: FilesystemCapabilityLeaseStore<InMemoryBackend>,
        /// Both concurrent auth_resume_json callers rendezvous here after
        /// `leases_for_scope` returns so they race on `begin_dispatch_claimed`.
        scan_barrier: StdArc<Barrier>,
        /// Armed once; only the first `leases_for_scope` call (during the
        /// concurrent race) should hit the barrier — later calls (cleanup,
        /// assertions) skip it.
        barrier_armed: std::sync::atomic::AtomicBool,
    }

    impl BarrierLeaseStore {
        fn new(barrier: StdArc<Barrier>) -> Self {
            Self {
                inner: in_memory_backed_capability_lease_store(),
                scan_barrier: barrier,
                barrier_armed: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn arm_barrier(&self) {
            self.barrier_armed
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl CapabilityLeaseStore for BarrierLeaseStore {
        async fn issue(
            &self,
            lease: CapabilityLease,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.issue(lease).await
        }

        async fn revoke(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.revoke(scope, lease_id).await
        }

        async fn get(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Option<CapabilityLease> {
            self.inner.get(scope, lease_id).await
        }

        async fn claim(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner
                .claim(scope, lease_id, invocation_fingerprint)
                .await
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.consume(scope, lease_id).await
        }

        async fn begin_dispatch_claimed(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner
                .begin_dispatch_claimed(scope, lease_id, invocation_fingerprint)
                .await
        }

        async fn abort_dispatch_claimed(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.abort_dispatch_claimed(scope, lease_id).await
        }

        async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
            let leases = self.inner.leases_for_scope(scope).await;
            // Only wait at the barrier when armed (during the concurrent race phase).
            if self.barrier_armed.load(std::sync::atomic::Ordering::SeqCst) {
                // Both concurrent callers have now completed their helper scan and
                // seen the Claimed lease.  Wait for both to arrive before either
                // proceeds to `begin_dispatch_claimed`.
                self.scan_barrier.wait().await;
            }
            leases
        }

        async fn active_leases_for_context(
            &self,
            context: &ExecutionContext,
        ) -> Vec<CapabilityLease> {
            self.inner.active_leases_for_context(context).await
        }
    }

    // ── Gating dispatcher ────────────────────────────────────────────────────
    // Call 0: resume_json bounce → AuthRequired (lease stays Claimed).
    // Call 1: winner auth_resume_json → signals `in_dispatch`, blocks on
    //         `release`, then succeeds.  Holds Dispatching while loser runs.
    struct GatingDispatcher {
        inner: TestDispatcher,
        call_count: std::sync::atomic::AtomicUsize,
        in_dispatch: StdArc<Notify>,
        release: StdArc<Notify>,
    }

    #[async_trait]
    impl CapabilityDispatcher for GatingDispatcher {
        async fn dispatch_json(
            &self,
            request: Authorized,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                let capability = request.invocation().capability.clone();
                let _ = request.abort();
                return Err(DispatchError::AuthRequired {
                    capability,
                    required_secrets: vec![],
                    credential_requirements: vec![],
                });
            }
            // Winner's call: signal we're in dispatch, then wait for release.
            self.in_dispatch.notify_one();
            self.release.notified().await;
            self.inner.dispatch_json(request).await
        }
    }

    let scan_barrier = StdArc::new(Barrier::new(2));
    let in_dispatch = StdArc::new(Notify::new());
    let release = StdArc::new(Notify::new());

    let registry = StdArc::new(registry_with_echo_capability());
    let gating_dispatcher = StdArc::new(GatingDispatcher {
        inner: recording_dispatcher(),
        call_count: std::sync::atomic::AtomicUsize::new(0),
        in_dispatch: StdArc::clone(&in_dispatch),
        release: StdArc::clone(&release),
    });
    let run_state = StdArc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests =
        StdArc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let leases = StdArc::new(BarrierLeaseStore::new(StdArc::clone(&scan_barrier)));

    // ── Phase 1: invoke → BlockedApproval ──────────────────────────────────
    let block_host = capability_host(&registry, &*gating_dispatcher, &ApprovalAuthorizer)
        .with_run_state(&*run_state)
        .with_approval_requests(&*approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "real concurrent dispatch reuse race"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // ── Phase 2: approve → Active lease ────────────────────────────────────
    ApprovalResolver::new(&*approval_requests, &*leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();

    // ── Phase 3: resume_json (call 0) → AuthRequired → lease Claimed ────────
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer = GrantAuthorizer::new();
    {
        let resume_host = capability_host(&registry, &*gating_dispatcher, &resume_authorizer)
            .with_run_state(&*run_state)
            .with_approval_requests(&*approval_requests)
            .with_capability_leases(&*leases);
        resume_host
            .resume_json(CapabilityResumeRequest {
                context: resume_context.clone(),
                approval_request_id: approval_id,
                capability_id: capability_id(),
                estimate: estimate.clone(),
                input: input.clone(),
            })
            .await
            .unwrap_err();
    }

    // Confirm the lease is now Claimed.
    let lease_id = leases
        .inner
        .leases_for_scope(&scope)
        .await
        .into_iter()
        .next()
        .expect("lease must exist after resume_json")
        .grant
        .id;
    assert_eq!(
        leases.inner.get(&scope, lease_id).await.unwrap().status,
        CapabilityLeaseStatus::Claimed,
        "pre-condition: lease must be Claimed after resume_json auth bounce"
    );

    // ── Phase 4: arm the barrier and spawn BOTH concurrent tasks ────────────
    // Both auth_resume_json calls use `leases_for_scope` inside
    // `matching_claimed_approval_lease_for_auth_resume`.  The barrier makes
    // both complete that scan before either proceeds to `begin_dispatch_claimed`,
    // creating the real data race on the Claimed→Dispatching transition.
    //
    // Both tasks are spawned so they can BOTH arrive at the barrier.  The one
    // that wins `begin_dispatch_claimed` (Claimed→Dispatching) then enters the
    // gating dispatcher and signals `in_dispatch`.  The other task (loser)
    // returns `InactiveLease { status: Dispatching }` and finishes quickly.
    leases.arm_barrier();

    let task_a_registry = StdArc::clone(&registry);
    let task_a_dispatcher = StdArc::clone(&gating_dispatcher);
    let task_a_run_state = StdArc::clone(&run_state);
    let task_a_approval_requests = StdArc::clone(&approval_requests);
    let task_a_leases = StdArc::clone(&leases);
    let task_a_authorizer = GrantAuthorizer::new();
    let mut task_a_context = original_context.clone();
    task_a_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let task_a_estimate = estimate.clone();
    let task_a_input = input.clone();

    let task_a = tokio::spawn(async move {
        let host = capability_host(&task_a_registry, &*task_a_dispatcher, &task_a_authorizer)
            .with_run_state(&*task_a_run_state)
            .with_approval_requests(&*task_a_approval_requests)
            .with_capability_leases(&*task_a_leases);
        host.auth_resume_json(CapabilityAuthResumeRequest {
            context: task_a_context,
            capability_id: capability_id(),
            estimate: task_a_estimate,
            input: task_a_input,
            approval_request_id: Some(approval_id),
        })
        .await
    });

    let task_b_registry = StdArc::clone(&registry);
    let task_b_dispatcher = StdArc::clone(&gating_dispatcher);
    let task_b_run_state = StdArc::clone(&run_state);
    let task_b_approval_requests = StdArc::clone(&approval_requests);
    let task_b_leases = StdArc::clone(&leases);
    let task_b_authorizer = GrantAuthorizer::new();
    let mut task_b_context = original_context.clone();
    task_b_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let task_b_estimate = estimate.clone();
    let task_b_input = input.clone();

    let task_b = tokio::spawn(async move {
        let host = capability_host(&task_b_registry, &*task_b_dispatcher, &task_b_authorizer)
            .with_run_state(&*task_b_run_state)
            .with_approval_requests(&*task_b_approval_requests)
            .with_capability_leases(&*task_b_leases);
        host.auth_resume_json(CapabilityAuthResumeRequest {
            context: task_b_context,
            capability_id: capability_id(),
            estimate: task_b_estimate,
            input: task_b_input,
            approval_request_id: Some(approval_id),
        })
        .await
    });

    // ── Phase 5: winner is in dispatcher, release and join both ─────────────
    // Wait until the winner has entered the gating dispatcher (holding
    // Dispatching state).  At this point the loser has already returned
    // InactiveLease{Dispatching} and its task has finished.
    in_dispatch.notified().await;

    // Release the winner so it can complete.
    release.notify_one();

    let result_a = task_a.await.expect("task_a must not panic");
    let result_b = task_b.await.expect("task_b must not panic");

    // ── Phase 6: assert exactly one winner and one loser ────────────────────
    // One task must have returned Ok (winner), the other must have returned
    // Err(Lease(InactiveLease { status: Dispatching })) (loser).
    assert!(
        result_a.is_ok() ^ result_b.is_ok(),
        "expected exactly one winner (Ok) and one loser (Err), got:\n  task_a={result_a:?}\n  task_b={result_b:?}"
    );

    let loser_err = if let Err(e) = result_a {
        e
    } else {
        result_b.unwrap_err()
    };
    assert!(
        matches!(
            &loser_err,
            CapabilityInvocationError::Lease(e)
            if matches!(
                e.as_ref(),
                CapabilityLeaseError::InactiveLease {
                    status: CapabilityLeaseStatus::Dispatching,
                    ..
                }
            )
        ),
        "loser must observe InactiveLease {{ status: Dispatching }}, got {loser_err:?}"
    );

    // Run must be Completed (winner finished dispatch).
    let run_final = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run_final.status,
        RunStatus::Completed,
        "run must be Completed after winner dispatch"
    );

    // The inner RecordingDispatcher sees exactly one successful dispatch.
    assert!(
        gating_dispatcher.inner.call_count() > 0,
        "exactly one successful dispatch must have been recorded by the winner"
    );
    assert_eq!(
        gating_dispatcher.inner.call_count(),
        1,
        "exactly one successful dispatch (loser never reached the dispatcher)"
    );
}

// ---------------------------------------------------------------------------
// Authorization Deny on an AlreadyClaimed (Dispatching) reuse lease: lease must be Revoked
//
// When `auth_resume_json` is called with a prior Claimed lease (AlreadyClaimed
// path), `begin_dispatch_claimed` transitions the lease Claimed→Dispatching
// BEFORE `authorize_dispatch_with_trust` runs in `dispatch_resumed_capability`.
//
// Pre-fix: if authorization returns Deny, the function returned early without
// touching the lease, leaving it stuck in Dispatching (burned / locked).
//
// Post-fix: the Deny arm revokes the AlreadyClaimed lease before returning,
// so the lease ends in Revoked (terminal), not Dispatching.
//
// Test drive: invoke → approve → resume_json (auth bounce) → lease is Claimed →
// auth_resume_json with DenyingAuthorizer → assert lease is Revoked.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_authorization_deny_revokes_dispatching_lease() {
    use async_trait::async_trait;
    use ironclaw_trust::TrustDecision as TrustDecisionAlias;

    struct DenyingAuthorizer;

    #[async_trait]
    impl TrustAwareCapabilityDispatchAuthorizer for DenyingAuthorizer {
        async fn authorize_dispatch_with_trust(
            &self,
            _context: &ExecutionContext,
            _descriptor: &CapabilityDescriptor,
            _estimate: &ResourceEstimate,
            _trust_decision: &TrustDecisionAlias,
        ) -> Decision {
            Decision::Deny {
                reason: DenyReason::MissingGrant,
            }
        }
    }

    // Dispatcher: first call (resume_json) → AuthRequired bounce; second call
    // (auth_resume_json) is never reached because the authorizer denies first.
    // `auth_required()` returns AuthRequired on every call, which covers this.
    let registry = registry_with_echo_capability();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // ── Phase 1: invoke → BlockedApproval ──────────────────────────────────
    let block_dispatcher = TestDispatcher::auth_required();
    let block_host = capability_host(&registry, &block_dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);

    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "deny-after-claimed test"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // ── Phase 2: approve → Active lease ────────────────────────────────────
    let issued = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();
    let lease_id = issued.grant.id;

    // ── Phase 3: resume_json → AuthRequired bounce → lease Claimed ──────────
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer = GrantAuthorizer::new();
    let resume_dispatcher = TestDispatcher::auth_required();
    let resume_host = capability_host(&registry, &resume_dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    resume_host
        .resume_json(CapabilityResumeRequest {
            context: resume_context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    // Pre-condition: lease must be Claimed after auth bounce.
    let lease_before = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_before.status,
        CapabilityLeaseStatus::Claimed,
        "pre-condition: lease must be Claimed after resume_json auth bounce"
    );

    // ── Phase 4: auth_resume_json with DenyingAuthorizer ────────────────────
    // This is the bug path: begin_dispatch_claimed (Claimed→Dispatching) runs
    // before authorize_dispatch_with_trust, then Deny is returned.
    // Pre-fix: lease stuck in Dispatching.
    // Post-fix: lease revoked → Revoked.
    let deny_authorizer = DenyingAuthorizer;
    let deny_dispatcher = recording_dispatcher();
    let deny_host = capability_host(&registry, &deny_dispatcher, &deny_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = deny_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(err, CapabilityInvocationError::AuthorizationDenied { .. }),
        "expected AuthorizationDenied, got {err:?}"
    );
    assert_eq!(
        deny_dispatcher.call_count(),
        0,
        "dispatch must not fire when authorization is denied"
    );

    // ── Core assertion: lease must be Revoked, not Dispatching ───────────────
    // Pre-fix: lease.status == Dispatching (burned/locked).
    // Post-fix: lease.status == Revoked.
    let lease_after = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after.status,
        CapabilityLeaseStatus::Revoked,
        "lease must be Revoked after authorization Deny in auth_resume_json \
         (pre-fix: lease was left stuck in Dispatching)"
    );
}

// ---------------------------------------------------------------------------
// Authorization RequireApproval on an AlreadyClaimed (Dispatching) reuse lease: lease must be Revoked
//
// Same invariant as the Deny test above, but the authorizer returns
// RequireApproval instead of Deny.  Both are terminal refusals in the context
// of a resumed invocation; neither should leave the lease stuck in Dispatching.
//
// Pre-fix: RequireApproval arm returned early without revoking, leaving the
// Dispatching lease permanently locked.
// Post-fix: RequireApproval arm revokes AlreadyClaimed lease before returning.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_authorization_require_approval_revokes_dispatching_lease() {
    // Dispatcher: always returns AuthRequired (used for the resume_json bounce).
    let always_auth_required = TestDispatcher::auth_required();

    let registry = registry_with_echo_capability();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // ── Phase 1: invoke → BlockedApproval ──────────────────────────────────
    let block_host = capability_host(&registry, &always_auth_required, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);

    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "require-approval-after-claimed test"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // ── Phase 2: approve → Active lease ────────────────────────────────────
    let issued = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();
    let lease_id = issued.grant.id;

    // ── Phase 3: resume_json → AuthRequired → lease Claimed ─────────────────
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_grant_authorizer = GrantAuthorizer::new();
    let resume_host = capability_host(&registry, &always_auth_required, &resume_grant_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    resume_host
        .resume_json(CapabilityResumeRequest {
            context: resume_context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    // Pre-condition: lease Claimed after auth bounce.
    let lease_before = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_before.status,
        CapabilityLeaseStatus::Claimed,
        "pre-condition: lease must be Claimed after resume_json auth bounce"
    );

    // ── Phase 4: auth_resume_json with ApprovalAuthorizer (RequireApproval) ──
    // ApprovalAuthorizer always returns Decision::RequireApproval.
    // Pre-fix: lease stuck Dispatching.
    // Post-fix: lease → Revoked.
    let approval_authorizer_for_resume = ApprovalAuthorizer;
    let recording_dispatcher = recording_dispatcher();
    let req_approval_host = capability_host(
        &registry,
        &recording_dispatcher,
        &approval_authorizer_for_resume,
    )
    .with_run_state(&run_state)
    .with_approval_requests(&approval_requests)
    .with_capability_leases(&leases);

    let err = req_approval_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::AuthorizationRequiresApproval { .. }
        ),
        "expected AuthorizationRequiresApproval, got {err:?}"
    );
    assert_eq!(
        recording_dispatcher.call_count(),
        0,
        "dispatch must not fire when authorization requires approval"
    );

    // ── Core assertion: lease must be Revoked, not Dispatching ───────────────
    // Pre-fix: lease.status == Dispatching (permanently locked).
    // Post-fix: lease.status == Revoked.
    let lease_after = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after.status,
        CapabilityLeaseStatus::Revoked,
        "lease must be Revoked after authorization RequireApproval in auth_resume_json \
         (pre-fix: lease was left stuck in Dispatching)"
    );
}

// ---------------------------------------------------------------------------
// TEST JYSbO: auth_resume_json returns ResumeStoreMissing { store: "run_state" }
// when the host is built WITHOUT a run_state store.
//
// The very first thing auth_resume_json does (host.rs ~778) is unwrap
// `self.run_state` via ok_or_else.  A host wired without `.with_run_state()`
// must surface ResumeStoreMissing immediately — before any dispatch or
// run-state transition.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_returns_store_missing_when_run_state_absent() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();

    // Host has NO run_state store — only the bare minimum.
    let host = capability_host(&registry, &dispatcher, &authorizer);
    // Do NOT call .with_run_state(...).

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: execution_context(CapabilitySet {
                grants: vec![dispatch_grant()],
            }),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "no run_state store"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ResumeStoreMissing { store, .. }
            if store == "run_state"
        ),
        "expected ResumeStoreMissing {{ store: \"run_state\" }}, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when run_state store is absent"
    );
}

// ---------------------------------------------------------------------------
// TEST JYSbP: auth_resume_json maps a missing run record to
// RunState(UnknownInvocation) when the store is wired but has no record for
// the invocation.
//
// host.rs ~795: run_state.get(...).await?.ok_or(RunStateError::UnknownInvocation)
// A run_state store that is present but empty must surface UnknownInvocation
// wrapped in CapabilityInvocationError::RunState — no dispatch, no run
// transition.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_unknown_invocation_when_run_record_missing() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    // No run record seeded — the store is empty.

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let host = capability_host(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "no run record seeded"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::RunState(ref e)
            if matches!(e.as_ref(), RunStateError::UnknownInvocation { .. })
        ),
        "expected RunState(UnknownInvocation), got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when no run record exists"
    );
}

// ---------------------------------------------------------------------------
// TEST JYSbQ: validate_approval_request_matches_invocation mismatch branches
//
// auth_resume_json calls validate_approval_request_matches_invocation (host.rs
// ~894) before the fingerprint check.  The validator checks three axes:
//   1. action — capability + estimate in the approval's Action::Dispatch
//   2. correlation_id — must match context.correlation_id
//   3. requested_by — must equal Principal::Extension(context.extension_id)
//
// Each axis below seeds an Approved approval with EXACTLY ONE field mismatched,
// then calls auth_resume_json and asserts:
//   (a) the specific ApprovalRequestMismatch { field } variant is returned
//   (b) no dispatch fires
//   (c) the run is Failed (fail_run_if_configured is called for all mismatches)
// ---------------------------------------------------------------------------

/// Helper: seed a run record in BlockedAuth and build a host wired with all
/// necessary stores for the approval-validation path.
async fn setup_blocked_auth_run_with_stores(
    run_state: &ironclaw_run_state::FilesystemRunStateStore<ironclaw_filesystem::InMemoryBackend>,
    approval_requests: &ironclaw_run_state::FilesystemApprovalRequestStore<
        ironclaw_filesystem::InMemoryBackend,
    >,
    leases: &FilesystemCapabilityLeaseStore<InMemoryBackend>,
    context: &ExecutionContext,
) {
    let scope = &context.resource_scope;
    let invocation_id = context.invocation_id;
    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();
    let _ = leases; // consumed by the caller via .with_capability_leases
    let _ = approval_requests; // consumed by the caller via .with_approval_requests
}

#[tokio::test]
async fn auth_resume_json_approval_request_mismatch_action() {
    // Approval references a DIFFERENT capability than the one being invoked.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    setup_blocked_auth_run_with_stores(&run_state, &approval_requests, &leases, &context).await;

    // Approval action references a different capability (action mismatch).
    let different_capability = CapabilityId::new("other.tool").unwrap();
    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
                action: Box::new(Action::Dispatch {
                    capability: different_capability,
                    estimated_resources: ResourceEstimate::default(),
                }),
                invocation_fingerprint: None,
                reason: "action mismatch test".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    approval_requests
        .approve(&scope, approval_id)
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "action mismatch"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ApprovalRequestMismatch {
                field: "action",
                ..
            }
        ),
        "expected ApprovalRequestMismatch {{ field: \"action\" }}, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire on action mismatch"
    );
    // fail_run_if_configured is called on all ApprovalRequestMismatch paths.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::Failed,
        "run must be Failed after action mismatch"
    );
}

#[tokio::test]
async fn auth_resume_json_approval_request_mismatch_correlation_id() {
    // Approval has a DIFFERENT correlation_id than the current invocation context.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    setup_blocked_auth_run_with_stores(&run_state, &approval_requests, &leases, &context).await;

    // Use a freshly-generated correlation_id so it cannot match context.correlation_id.
    let different_correlation_id = CorrelationId::new();
    assert_ne!(
        different_correlation_id, context.correlation_id,
        "pre-condition: fresh CorrelationId must differ from the context one"
    );

    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: different_correlation_id,
                requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: ResourceEstimate::default(),
                }),
                invocation_fingerprint: None,
                reason: "correlation_id mismatch test".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    approval_requests
        .approve(&scope, approval_id)
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "correlation_id mismatch"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ApprovalRequestMismatch {
                field: "correlation_id",
                ..
            }
        ),
        "expected ApprovalRequestMismatch {{ field: \"correlation_id\" }}, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire on correlation_id mismatch"
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::Failed,
        "run must be Failed after correlation_id mismatch"
    );
}

#[tokio::test]
async fn auth_resume_json_approval_request_mismatch_requested_by() {
    // Approval was requested_by Principal::HostRuntime, but the validator
    // expects Principal::Extension(context.extension_id) — so requested_by
    // must mismatch.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    setup_blocked_auth_run_with_stores(&run_state, &approval_requests, &leases, &context).await;

    // Use HostRuntime as requested_by; validator expects Extension("caller").
    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: context.correlation_id,
                requested_by: Principal::HostRuntime,
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: ResourceEstimate::default(),
                }),
                invocation_fingerprint: None,
                reason: "requested_by mismatch test".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    approval_requests
        .approve(&scope, approval_id)
        .await
        .unwrap();

    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "requested_by mismatch"}),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::ApprovalRequestMismatch {
                field: "requested_by",
                ..
            }
        ),
        "expected ApprovalRequestMismatch {{ field: \"requested_by\" }}, got {err:?}"
    );
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire on requested_by mismatch"
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::Failed,
        "run must be Failed after requested_by mismatch"
    );
}

// ---------------------------------------------------------------------------
// Concurrent auth-resume: FRESH Active-lease path — two concurrent callers
// where the lease starts ACTIVE.
//
// PRE-FIX BUG: The fresh branch called claim() (Active→Claimed) but left the
// lease Claimed during dispatch.  A concurrent caller that arrived after
// claim() but before dispatch could find the Claimed lease via the REUSE
// branch, call begin_dispatch_claimed (Claimed→Dispatching), and also dispatch
// — double execution.
//
// POST-FIX: The fresh branch immediately calls begin_dispatch_claimed after
// claim() (Active→Claimed→Dispatching).  A concurrent caller that arrives in
// the window between claim() and begin_dispatch_claimed:
//   - matching_approval_lease → None (lease is Claimed, not Active)
//   - matching_claimed_approval_lease_for_auth_resume → Some(Claimed)
//   - begin_dispatch_claimed → Err(InactiveLease{Dispatching}) because the
//     fresh-path winner already advanced it
//   - claim_error_may_be_concurrent_resume → true → warn, no fail_run
//   - returns Err(Lease(InactiveLease{Dispatching}))
// Only ONE dispatch is recorded.
//
// Race choreography:
//   1. invoke → approve → block_auth directly (lease stays Active).
//   2. Spawn task A (fresh path). A calls claim() — a GatedLeaseStore holds A
//      INSIDE claim() after the CAS so the lease is Claimed while A is parked.
//   3. Main task runs B while A is parked: B takes the REUSE path
//      (matching_approval_lease → None, lease Claimed) → begin_dispatch_claimed
//      → Dispatching → B dispatches → Ok (B is WINNER).
//   4. Release A from the gate. A exits claim() and:
//      PRE-FIX:  A has no begin_dispatch_claimed call; A's
//                dispatch_resumed_capability receives AlreadyClaimed(Claimed
//                snapshot) and dispatches → dispatch_count == 2 (RED).
//      POST-FIX: A calls begin_dispatch_claimed → sees Dispatching →
//                Err(InactiveLease{Dispatching}) → no fail_run →
//                A returns Err(Lease(...)) → dispatch_count == 1 (GREEN).
//
// Assertions (post-fix / green):
//   - B (winner): Ok, lease Consumed after B's dispatch.
//   - A (loser): Err(Lease(_)), run stays Completed (B finished it), A did
//     not additionally fail the run.
//   - dispatch_count == 1.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_auth_resume_fresh_active_lease_loser_does_not_double_dispatch() {
    use async_trait::async_trait;
    use std::sync::Arc as StdArc;
    use tokio::sync::Notify;

    // ── GatedLeaseStore ──────────────────────────────────────────────────────
    // After the first armed claim() CAS (Active→Claimed) succeeds in the inner
    // store, suspends the caller until `claim_release` is notified.  Any other
    // caller that runs `matching_approval_lease` during this window finds None
    // (lease is Claimed, not Active) and falls into the REUSE branch.
    // Synchronization primitive, not an I/O fault — cannot move to
    // ironclaw_filesystem::FaultInjecting: the Notify gate suspends a caller
    // mid-`claim` to force an interleaving; FaultInjecting only injects errors +
    // records ops and is explicitly not a synchronization barrier.
    struct GatedLeaseStore {
        inner: FilesystemCapabilityLeaseStore<InMemoryBackend>,
        claim_entered: StdArc<Notify>,
        claim_release: StdArc<Notify>,
        armed: std::sync::atomic::AtomicBool,
    }

    impl GatedLeaseStore {
        fn new(claim_entered: StdArc<Notify>, claim_release: StdArc<Notify>) -> Self {
            Self {
                inner: in_memory_backed_capability_lease_store(),
                claim_entered,
                claim_release,
                armed: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn arm(&self) {
            self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl CapabilityLeaseStore for GatedLeaseStore {
        async fn issue(
            &self,
            lease: CapabilityLease,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.issue(lease).await
        }

        async fn revoke(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.revoke(scope, lease_id).await
        }

        async fn get(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Option<CapabilityLease> {
            self.inner.get(scope, lease_id).await
        }

        async fn claim(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            let result = self
                .inner
                .claim(scope, lease_id, invocation_fingerprint)
                .await;
            // Fire the gate on the first armed successful claim: lease is now
            // Claimed in the store but the caller hasn't returned from claim()
            // yet (and therefore hasn't called begin_dispatch_claimed).
            if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) && result.is_ok() {
                self.claim_entered.notify_one();
                self.claim_release.notified().await;
            }
            result
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.consume(scope, lease_id).await
        }

        async fn begin_dispatch_claimed(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
            invocation_fingerprint: &InvocationFingerprint,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner
                .begin_dispatch_claimed(scope, lease_id, invocation_fingerprint)
                .await
        }

        async fn abort_dispatch_claimed(
            &self,
            scope: &ResourceScope,
            lease_id: CapabilityGrantId,
        ) -> Result<CapabilityLease, CapabilityLeaseError> {
            self.inner.abort_dispatch_claimed(scope, lease_id).await
        }

        async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
            self.inner.leases_for_scope(scope).await
        }

        async fn active_leases_for_context(
            &self,
            context: &ExecutionContext,
        ) -> Vec<CapabilityLease> {
            self.inner.active_leases_for_context(context).await
        }
    }

    let claim_entered = StdArc::new(Notify::new());
    let claim_release = StdArc::new(Notify::new());

    let registry = StdArc::new(registry_with_echo_capability());
    let dispatcher = StdArc::new(recording_dispatcher());
    let run_state = StdArc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests =
        StdArc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let leases = StdArc::new(GatedLeaseStore::new(
        StdArc::clone(&claim_entered),
        StdArc::clone(&claim_release),
    ));

    // ── Phase 1: invoke → BlockedApproval ──────────────────────────────────
    let block_host = capability_host(&registry, &*dispatcher, &ApprovalAuthorizer)
        .with_run_state(&*run_state)
        .with_approval_requests(&*approval_requests);
    let original_context = execution_context(CapabilitySet::default());
    let scope = original_context.resource_scope.clone();
    let invocation_id = original_context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "fresh active lease concurrent race"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: original_context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();

    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    // ── Phase 2: approve → Active lease ─────────────────────────────────────
    let issued = ApprovalResolver::new(&*approval_requests, &*leases)
        .approve_dispatch(&scope, approval_id, dispatch_lease_approval())
        .await
        .unwrap();
    let lease_id = issued.grant.id;

    // ── Phase 3: block at auth directly (lease stays Active) ─────────────────
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    assert_eq!(
        leases.inner.get(&scope, lease_id).await.unwrap().status,
        CapabilityLeaseStatus::Active,
        "pre-condition: lease must be Active before the concurrent race"
    );

    // ── Phase 4: arm gate and spawn task A (FRESH Active path) ───────────────
    // A will be suspended inside claim() once the CAS succeeds.
    leases.arm();

    let mut task_a_context = original_context.clone();
    task_a_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let task_a_registry = StdArc::clone(&registry);
    let task_a_dispatcher = StdArc::clone(&dispatcher);
    let task_a_run_state = StdArc::clone(&run_state);
    let task_a_approval_requests = StdArc::clone(&approval_requests);
    let task_a_leases = StdArc::clone(&leases);
    let task_a_authorizer = GrantAuthorizer::new();
    let task_a_estimate = estimate.clone();
    let task_a_input = input.clone();

    let task_a = tokio::spawn(async move {
        let host = capability_host(&task_a_registry, &*task_a_dispatcher, &task_a_authorizer)
            .with_run_state(&*task_a_run_state)
            .with_approval_requests(&*task_a_approval_requests)
            .with_capability_leases(&*task_a_leases);
        host.auth_resume_json(CapabilityAuthResumeRequest {
            context: task_a_context,
            capability_id: capability_id(),
            estimate: task_a_estimate,
            input: task_a_input,
            approval_request_id: Some(approval_id),
        })
        .await
    });

    // Wait until A has claimed (Active→Claimed) and is parked inside claim().
    claim_entered.notified().await;

    // ── Phase 5: run B (REUSE path) while A is parked ────────────────────────
    // B: matching_approval_lease → None (lease is Claimed) →
    //    matching_claimed_approval_lease_for_auth_resume → Some(Claimed) →
    //    begin_dispatch_claimed (Claimed→Dispatching) → dispatch → Ok.
    // B is the WINNER.
    let mut winner_context = original_context.clone();
    winner_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let winner_result = capability_host(&registry, &*dispatcher, &GrantAuthorizer::new())
        .with_run_state(&*run_state)
        .with_approval_requests(&*approval_requests)
        .with_capability_leases(&*leases)
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: winner_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            approval_request_id: Some(approval_id),
        })
        .await;

    winner_result.unwrap_or_else(|e| panic!("concurrent winner (B) must succeed, got {e:?}"));

    // Winner B has dispatched and consumed the lease.
    assert_eq!(
        leases.inner.get(&scope, lease_id).await.unwrap().status,
        CapabilityLeaseStatus::Consumed,
        "lease must be Consumed after winner B dispatched"
    );
    assert_eq!(
        dispatcher.call_count(),
        1,
        "exactly one dispatch after winner B — before releasing A"
    );

    // ── Phase 6: release A from the gate ─────────────────────────────────────
    // POST-FIX: A calls begin_dispatch_claimed immediately after claim()
    //   returns.  The lease is now Dispatching (winner advanced it) or already
    //   Consumed.  Both statuses are InactiveLease variants that
    //   claim_error_may_be_concurrent_resume recognises → warn, no fail_run,
    //   return Err(Lease(...)).
    // PRE-FIX: A has no begin_dispatch_claimed in the preamble; A enters
    //   dispatch_resumed_capability with AlreadyClaimed(Claimed-snapshot) and
    //   dispatches → dispatch_count == 2 (RED).
    claim_release.notify_one();
    let loser_result = task_a.await.expect("task_a must not panic");

    // ── Assertions (post-fix / green) ────────────────────────────────────────
    // A must return Err(Lease(_)) — the concurrent fresh-path loser.
    let loser_err = loser_result.unwrap_err();
    assert!(
        matches!(loser_err, CapabilityInvocationError::Lease(_)),
        "fresh-path concurrent loser (A) must return a Lease error, got {loser_err:?} \
         (pre-fix: A would have returned Ok and dispatched again)"
    );

    // A must NOT have dispatched.
    assert_eq!(
        dispatcher.call_count(),
        1,
        "exactly one dispatch must be recorded — A must not have dispatched \
         (pre-fix: dispatch_count would be 2)"
    );

    // Run must be Completed (B finished it; A did not additionally fail it).
    let run_final = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run_final.status,
        RunStatus::Completed,
        "run must remain Completed after loser A's concurrent Lease error"
    );
}

// ---------------------------------------------------------------------------
// UnknownCapability must not strand the approval lease
//
// BUG (ordering hole): In the pre-fix code, `auth_resume_json` acquires and
// transitions the approval lease (Active→Claimed→Dispatching or
// Claimed→Dispatching) BEFORE it checks whether the capability still exists
// in the registry via `self.registry.get_capability(...)`.  If the capability
// is gone (unregistered between the original invocation and the resume), the
// `UnknownCapability` early-return fires AFTER the lease was already mutated,
// leaving a one-shot approval lease permanently stranded in Claimed/Dispatching.
//
// FIX (reorder): Move `self.registry.get_capability(...)` to BEFORE the
// approval-lease acquisition block so that an unknown capability returns
// `UnknownCapability` WITHOUT ever touching the lease.
//
// This test covers TWO sub-cases:
//   (a) Active-lease path — the approval was just issued; lease is Active.
//       Pre-fix: lease is left Claimed (after claim()) or Dispatching (after
//       begin_dispatch_claimed()).
//       Post-fix: lease remains Active (untouched).
//   (b) Claimed-lease path (reuse) — resume_json previously bounced at auth;
//       lease is Claimed.
//       Pre-fix: lease is left Dispatching (after begin_dispatch_claimed()).
//       Post-fix: lease remains Claimed (untouched).
//
// For each sub-case we assert:
//   1. auth_resume_json returns Err(UnknownCapability).
//   2. The approval lease is NOT in Claimed or Dispatching state — i.e. it is
//      still in whatever state it was seeded in (Active or Claimed).
//   3. No dispatch was attempted.
// ---------------------------------------------------------------------------

// Sub-case (a): Active lease — registry missing capability
#[tokio::test]
async fn auth_resume_json_unknown_capability_does_not_strand_active_approval_lease() {
    // Use a registry that does NOT contain the echo.say capability — simulates
    // a capability that was unregistered between the original invocation and
    // the auth resume.
    let empty_registry = ironclaw_extensions::ExtensionRegistry::new();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    // Seed the run in BlockedAuth state (as it would be after a prior auth gate).
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "unknown capability lease strand test"});

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Issue an Active approval lease for this invocation (fingerprinted to the
    // exact estimate + input we will pass to auth_resume_json).
    let invocation_fingerprint =
        InvocationFingerprint::for_dispatch(&scope, &capability_id(), &estimate, &input).unwrap();
    let lease = CapabilityLease {
        grant: CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id(),
            grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: vec![],
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        },
        scope: scope.clone(),
        invocation_fingerprint: Some(invocation_fingerprint.clone()),
        status: CapabilityLeaseStatus::Active,
    };
    let issued_lease = leases.issue(lease).await.unwrap();
    let lease_id = issued_lease.grant.id;

    // Seed the approval as Approved in the store so the approval-validation
    // path passes before reaching the lease-acquisition block.
    // requested_by must match Principal::Extension(context.extension_id) per
    // validate_approval_request_matches_invocation.
    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: Some(invocation_fingerprint),
                reason: "approved".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    approval_requests
        .approve(&scope, approval_id)
        .await
        .unwrap();

    // Host is wired with the EMPTY registry (capability not registered).
    let host = capability_host(&empty_registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input,
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    // 1. Must return UnknownCapability.
    assert!(
        matches!(err, CapabilityInvocationError::UnknownCapability { .. }),
        "expected UnknownCapability when capability is not registered, got {err:?}"
    );

    // 2. Lease must NOT be stranded in Claimed or Dispatching.
    //    Post-fix: still Active (untouched because the check fires first).
    //    Pre-fix: Claimed or Dispatching (lease was mutated before the check).
    let lease_after = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after.status,
        CapabilityLeaseStatus::Active,
        "lease must remain Active when auth_resume_json returns UnknownCapability \
         (pre-fix: lease was left Claimed/Dispatching because the capability check \
         came AFTER the lease acquisition block)"
    );

    // 3. No dispatch must have been attempted.
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when capability is unknown"
    );
}

// Sub-case (b): Claimed lease (reuse path) — registry missing capability
#[tokio::test]
async fn auth_resume_json_unknown_capability_does_not_strand_claimed_approval_lease() {
    // Same setup as sub-case (a) except the lease starts in the Claimed state
    // (as it would be after a prior resume_json auth bounce left it Claimed).
    let empty_registry = ironclaw_extensions::ExtensionRegistry::new();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let leases = in_memory_backed_capability_lease_store();

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "claimed lease strand test"});

    run_state
        .start(RunStart {
            invocation_id,
            scope: scope.clone(),
            capability_id: capability_id(),
            authenticated_actor_user_id: None,
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let invocation_fingerprint =
        InvocationFingerprint::for_dispatch(&scope, &capability_id(), &estimate, &input).unwrap();

    // Issue the lease as Active then immediately claim it to put it in the
    // Claimed state (mimicking what resume_json leaves behind).
    let lease = CapabilityLease {
        grant: CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id(),
            grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: vec![],
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        },
        scope: scope.clone(),
        invocation_fingerprint: Some(invocation_fingerprint.clone()),
        status: CapabilityLeaseStatus::Active,
    };
    let issued_lease = leases.issue(lease).await.unwrap();
    let lease_id = issued_lease.grant.id;
    // Advance to Claimed (simulating resume_json having claimed it before
    // the auth bounce occurred).
    leases
        .claim(&scope, lease_id, &invocation_fingerprint)
        .await
        .unwrap();

    // Verify precondition: lease is Claimed.
    assert_eq!(
        leases.get(&scope, lease_id).await.unwrap().status,
        CapabilityLeaseStatus::Claimed,
        "precondition: lease must be Claimed before calling auth_resume_json"
    );

    let approval_id = ApprovalRequestId::new();
    approval_requests
        .save_pending(
            scope.clone(),
            ApprovalRequest {
                id: approval_id,
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: Some(invocation_fingerprint),
                reason: "approved".to_string(),
                reusable_scope: None,
            },
        )
        .await
        .unwrap();
    approval_requests
        .approve(&scope, approval_id)
        .await
        .unwrap();

    let host = capability_host(&empty_registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input,
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap_err();

    // 1. Must return UnknownCapability.
    assert!(
        matches!(err, CapabilityInvocationError::UnknownCapability { .. }),
        "expected UnknownCapability when capability is not registered, got {err:?}"
    );

    // 2. Lease must NOT be stranded in Dispatching.
    //    Post-fix: still Claimed (untouched because the check fires first).
    //    Pre-fix: Dispatching (begin_dispatch_claimed was called before the check).
    let lease_after = leases.get(&scope, lease_id).await.unwrap();
    assert_eq!(
        lease_after.status,
        CapabilityLeaseStatus::Claimed,
        "lease must remain Claimed when auth_resume_json returns UnknownCapability \
         (pre-fix: lease was left Dispatching because begin_dispatch_claimed ran \
         before the capability existence check)"
    );

    // 3. No dispatch.
    assert_eq!(
        dispatcher.call_count(),
        0,
        "dispatch must not fire when capability is unknown"
    );
}
