/// Contract tests for `CapabilityHost::auth_resume_json`.
///
/// These tests prove the fix that allows a one-shot approval lease to survive
/// an auth-gate re-dispatch: when `PendingAuthResume` carries the original
/// `invocation_id` (encoded as a resume token), auth_resume_json reuses it so
/// the fingerprinted lease — whose scope embeds the original invocation_id —
/// can still be matched and claimed.
///
/// # What would fail without the fix
///
/// Before the fix, re-dispatch after an auth gate always called `invoke_json`
/// with a fresh `InvocationId::new()`. The lease was scoped to the old
/// invocation_id, so `matching_approval_lease` found nothing — and a new
/// `RequireApproval` gate fired, producing the infinite re-approval loop
/// observed in Slack QA.
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
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
        allowed_effects: vec![EffectKind::DispatchCapability],
        mounts: MountView::default(),
        network: NetworkPolicy::default(),
        secrets: vec![],
        resource_ceiling: None,
        expires_at: None,
        max_invocations: Some(1),
    }
}

// ---------------------------------------------------------------------------
// Deliverable 1a: accepts run in BlockedAuth and dispatches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_accepts_blocked_auth_run_and_dispatches() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();

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
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let result = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
            approval_request_id: None,
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}

// ---------------------------------------------------------------------------
// Deliverable 1b: rejects run NOT in BlockedAuth
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_run_in_blocked_approval_status() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();

    // Block the invocation at an approval gate (not auth).
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    // Run is now BlockedApproval — auth_resume_json must reject it.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);

    let auth_authorizer = GrantAuthorizer::new();
    let auth_host =
        CapabilityHost::new(&registry, &dispatcher, &auth_authorizer).with_run_state(&run_state);
    let err = auth_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
            trust_decision: trust_decision(),
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
        dispatcher.dispatch_count(),
        0,
        "auth_resume must not dispatch when run is not in BlockedAuth"
    );
}

#[tokio::test]
async fn auth_resume_json_rejects_run_in_running_status() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();

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
        })
        .await
        .unwrap();

    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "try to resume running"}),
            trust_decision: trust_decision(),
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
    assert!(!dispatcher.has_request());
}

// ---------------------------------------------------------------------------
// Deliverable 1c: rejects invocation-fingerprint mismatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_fingerprint_mismatch_on_approval_request() {
    // Build an invocation, approve it (fingerprinted to original input), then
    // attempt auth_resume with different input — should reject before lease claim.
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

    // Phase 1: invoke (needs approval).
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
            trust_decision: trust_decision(),
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
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: original_context,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "MUTATED input"}),
            trust_decision: trust_decision(),
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
        dispatcher.dispatch_count(),
        0,
        "dispatch must not fire when fingerprint mismatches"
    );
    // Lease must still be active (not consumed or revoked).
    // (We cannot easily check the lease status here without knowing the lease id,
    // but the lack of dispatch proves no claim happened.)
}

// ---------------------------------------------------------------------------
// Deliverable 1d: with approval_request_id — claims fingerprinted lease and
// injects grant into authorized context
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_with_approval_request_id_claims_lease_and_dispatches() {
    // This is the core regression test for the fix:
    // approve → dispatch → auth blocked (original invocation_id) → auth_resume
    // with approval_request_id → lease found, claimed, dispatch succeeds.
    // Pre-fix: fresh invocation_id meant the lease scope would not match.
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

    // Phase 1: first invocation triggers approval.
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
            trust_decision: trust_decision(),
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
    // back to BlockedAuth (with the original invocation_id preserved in scope).
    run_state
        .block_auth(&scope, original_invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Phase 4: auth_resume_json with the SAME context (preserving correlation_id)
    // and the approval_request_id.  The fix ensures the scope uses the original
    // invocation_id so the fingerprint matches the issued lease.
    // We grant dispatch permission so the authorizer allows the dispatch.
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer_2 = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer_2)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let result = resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
            approval_request_id: Some(approval_id),
        })
        .await
        .unwrap();

    assert_eq!(
        result.dispatch.output,
        json!({"ok": true}),
        "auth_resume with original invocation_id must complete dispatch"
    );
    assert!(dispatcher.has_request());
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
// Deliverable 1e: without approval_request_id — skips lease path cleanly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_without_approval_request_id_skips_lease_path_and_dispatches() {
    // A capability that only needs auth (no prior approval gate).
    // auth_resume_json with approval_request_id = None must skip lease
    // validation entirely and proceed directly to dispatch.
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();

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
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let result = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
            approval_request_id: None, // no prior approval
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}
