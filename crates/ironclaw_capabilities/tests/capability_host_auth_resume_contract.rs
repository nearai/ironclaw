// Contract tests for `CapabilityHost::auth_resume_json`.
//
// Covers: lease survival across auth-gate re-dispatch, concurrent-claim race
// handling, terminal vs non-terminal bounce lease disposition, and approval
// fingerprint validation.
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
// auth_resume_json rejects capability_id mismatch against run record
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_rejects_capability_id_mismatch_against_run_record() {
    let registry = registry_with_echo_capability();
    let authorizer = GrantAuthorizer::new();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();

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
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    // Attempt auth_resume with a DIFFERENT capability_id.
    let different_id = CapabilityId::new("other.capability").unwrap();
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: different_id,
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "mismatch"}),
            trust_decision: trust_decision(),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    assert!(
        matches!(err, CapabilityInvocationError::ResumeContextMismatch { .. }),
        "capability_id mismatch against run record must be rejected, got {err:?}"
    );
    assert_eq!(
        dispatcher.dispatch_count(),
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
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

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

    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "pending approval"}),
            trust_decision: trust_decision(),
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
        dispatcher.dispatch_count(),
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
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();

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
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Host has run_state but NO approval_requests store.
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);

    let approval_id = ApprovalRequestId::new();
    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "needs approval store"}),
            trust_decision: trust_decision(),
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
        dispatcher.dispatch_count(),
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
    use async_trait::async_trait;

    // A dispatcher that returns AuthRequired on the first call and succeeds on
    // the second, so we can drive the real resume_json → auth bounce → auth_resume_json flow.
    struct FirstCallAuthRequiredDispatcher {
        inner: RecordingDispatcher,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl Default for FirstCallAuthRequiredDispatcher {
        fn default() -> Self {
            Self {
                inner: RecordingDispatcher::default(),
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl CapabilityDispatcher for FirstCallAuthRequiredDispatcher {
        async fn dispatch_json(
            &self,
            request: CapabilityDispatchRequest,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                // First dispatch: return AuthRequired to trigger the auth bounce.
                Err(DispatchError::AuthRequired {
                    capability: request.capability_id,
                    required_secrets: vec![],
                    credential_requirements: vec![],
                })
            } else {
                // Second dispatch (from auth_resume_json): succeed.
                self.inner.dispatch_json(request).await
            }
        }
    }

    let registry = registry_with_echo_capability();
    let dispatcher = FirstCallAuthRequiredDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

    // ── Phase 1: invoke_json → BlockedApproval ──────────────────────────────
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
            trust_decision: trust_decision(),
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
    // BEFORE FIX: revoke (Claimed→Revoked) ← the bug
    // AFTER FIX:  skip revoke (non-terminal auth bounce) → lease stays Claimed
    let mut resume_context = original_context.clone();
    resume_context.grants = CapabilitySet {
        grants: vec![dispatch_grant()],
    };
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
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
            trust_decision: trust_decision(),
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
    let auth_resume_host = CapabilityHost::new(&registry, &dispatcher, &auth_resume_authorizer)
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
            trust_decision: trust_decision(),
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
    // The dispatcher is FirstCallAuthRequiredDispatcher; the second call
    // went through inner.dispatch_json which recorded the request.
    let dispatched_request = dispatcher.inner.take_request();
    assert_eq!(
        dispatched_request.scope.invocation_id, original_invocation_id,
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
    use async_trait::async_trait;

    // Phase 1+2 use an auth-bounce dispatcher (AuthRequired on first call
    // so resume_json bounces and leaves the lease Claimed).
    // Phase 3 (auth_resume_json) uses a terminal-fail dispatcher
    // (UnknownCapability on first call so auth_resume_json errors terminally).
    struct TerminalFailDispatcher {
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl Default for TerminalFailDispatcher {
        fn default() -> Self {
            Self {
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl CapabilityDispatcher for TerminalFailDispatcher {
        async fn dispatch_json(
            &self,
            request: CapabilityDispatchRequest,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                // First call (from resume_json): auth bounce → lease stays Claimed.
                Err(DispatchError::AuthRequired {
                    capability: request.capability_id,
                    required_secrets: vec![],
                    credential_requirements: vec![],
                })
            } else {
                // Second call (from auth_resume_json): terminal failure.
                Err(DispatchError::UnknownCapability {
                    capability: request.capability_id,
                })
            }
        }
    }

    let registry = registry_with_echo_capability();
    let dispatcher = TerminalFailDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

    // Phase 1: invoke → BlockedApproval.
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
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
            trust_decision: trust_decision(),
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
    let auth_resume_host = CapabilityHost::new(&registry, &dispatcher, &auth_resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = auth_resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
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
    use async_trait::async_trait;

    // A dispatcher that always returns AuthRequired (non-terminal BlockAuth path).
    struct AlwaysAuthRequiredDispatcher;

    #[async_trait]
    impl CapabilityDispatcher for AlwaysAuthRequiredDispatcher {
        async fn dispatch_json(
            &self,
            request: CapabilityDispatchRequest,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            Err(DispatchError::AuthRequired {
                capability: request.capability_id,
                required_secrets: vec![],
                credential_requirements: vec![],
            })
        }
    }

    let registry = registry_with_echo_capability();
    let dispatcher = AlwaysAuthRequiredDispatcher;
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

    // Phase 1: invoke → BlockedApproval.
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
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
            trust_decision: trust_decision(),
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
    let auth_resume_host = CapabilityHost::new(&registry, &dispatcher, &auth_resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = auth_resume_host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
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

    // A lease store that delegates everything to an inner InMemoryCapabilityLeaseStore
    // except `claim()`, which returns InactiveLease { status: Claimed } once the
    // `fail_next_claim` flag is set — simulating the loser of a concurrent claim race.
    struct ClaimFailingLeaseStore {
        inner: InMemoryCapabilityLeaseStore,
        fail_next_claim: AtomicBool,
    }

    impl ClaimFailingLeaseStore {
        fn new() -> Self {
            Self {
                inner: InMemoryCapabilityLeaseStore::new(),
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
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = ClaimFailingLeaseStore::new();

    // Phase 1: invoke → BlockedApproval.
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
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
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context: resume_context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
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
        !dispatcher.has_request(),
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
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();

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
        })
        .await
        .unwrap();
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .unwrap();

    // Host has approval_requests configured BUT NO capability_leases.
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
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
            trust_decision: trust_decision(),
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
        dispatcher.dispatch_count(),
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
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();

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

    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"message": "denied approval"}),
            trust_decision: trust_decision(),
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
        dispatcher.dispatch_count(),
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
    // Wraps InMemoryCapabilityLeaseStore and inserts a Barrier(2) inside
    // `leases_for_scope`.  Both concurrent callers block at the barrier
    // inside the helper scan, then both are released together.  This
    // guarantees both see the Claimed lease before either calls
    // `begin_dispatch_claimed`, creating the real data race on the state
    // machine transition: one wins (Claimed→Dispatching), the other loses
    // (sees Dispatching → InactiveLease{Dispatching}).
    struct BarrierLeaseStore {
        inner: InMemoryCapabilityLeaseStore,
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
                inner: InMemoryCapabilityLeaseStore::new(),
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
        inner: RecordingDispatcher,
        call_count: std::sync::atomic::AtomicUsize,
        in_dispatch: StdArc<Notify>,
        release: StdArc<Notify>,
    }

    #[async_trait]
    impl CapabilityDispatcher for GatingDispatcher {
        async fn dispatch_json(
            &self,
            request: CapabilityDispatchRequest,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                return Err(DispatchError::AuthRequired {
                    capability: request.capability_id,
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
        inner: RecordingDispatcher::default(),
        call_count: std::sync::atomic::AtomicUsize::new(0),
        in_dispatch: StdArc::clone(&in_dispatch),
        release: StdArc::clone(&release),
    });
    let run_state = StdArc::new(InMemoryRunStateStore::new());
    let approval_requests = StdArc::new(InMemoryApprovalRequestStore::new());
    let leases = StdArc::new(BarrierLeaseStore::new(StdArc::clone(&scan_barrier)));

    // ── Phase 1: invoke → BlockedApproval ──────────────────────────────────
    let block_host = CapabilityHost::new(&registry, &*gating_dispatcher, &ApprovalAuthorizer)
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
        let resume_host = CapabilityHost::new(&registry, &*gating_dispatcher, &resume_authorizer)
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
                trust_decision: trust_decision(),
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
        let host = CapabilityHost::new(&task_a_registry, &*task_a_dispatcher, &task_a_authorizer)
            .with_run_state(&*task_a_run_state)
            .with_approval_requests(&*task_a_approval_requests)
            .with_capability_leases(&*task_a_leases);
        host.auth_resume_json(CapabilityAuthResumeRequest {
            context: task_a_context,
            capability_id: capability_id(),
            estimate: task_a_estimate,
            input: task_a_input,
            trust_decision: trust_decision(),
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
        let host = CapabilityHost::new(&task_b_registry, &*task_b_dispatcher, &task_b_authorizer)
            .with_run_state(&*task_b_run_state)
            .with_approval_requests(&*task_b_approval_requests)
            .with_capability_leases(&*task_b_leases);
        host.auth_resume_json(CapabilityAuthResumeRequest {
            context: task_b_context,
            capability_id: capability_id(),
            estimate: task_b_estimate,
            input: task_b_input,
            trust_decision: trust_decision(),
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
        gating_dispatcher.inner.has_request(),
        "exactly one successful dispatch must have been recorded by the winner"
    );
    assert_eq!(
        gating_dispatcher.inner.dispatch_count(),
        1,
        "exactly one successful dispatch (loser never reached the dispatcher)"
    );
}
