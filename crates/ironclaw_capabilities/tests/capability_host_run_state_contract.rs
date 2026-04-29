use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_run_state::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_blocks_for_approval_without_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs approval"});

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    assert!(!dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);
    let approval_id = run.approval_request_id.unwrap();
    let approval = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(
        approval.request.invocation_fingerprint,
        Some(
            InvocationFingerprint::for_dispatch(&scope, &capability_id(), &estimate, &input)
                .unwrap()
        )
    );
}

#[tokio::test]
async fn capability_host_marks_run_failed_when_obligations_are_unsupported() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &ObligatingAuthorizer)
        .with_run_state(&run_state);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked obligation"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("UnsupportedObligations"));
}

#[tokio::test]
async fn capability_host_returns_business_error_when_run_state_fail_transition_fails() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAuthorizer::new();
    let run_state = FailOnFailRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "denied"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::MissingGrant,
            ..
        }
    ));
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_does_not_orphan_approval_when_run_block_fails() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = FailBlockApprovalRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CapabilityInvocationError::RunState(_)));
    assert!(!dispatcher.has_request());
    assert!(
        approval_requests
            .records_for_scope(&scope)
            .await
            .unwrap()
            .is_empty()
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ApprovalBlock"));
}

#[tokio::test]
async fn capability_host_returns_specific_error_for_authorizer_fingerprint_mismatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &MismatchedApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "real input"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
    ));
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_returns_dispatch_result_when_run_completion_fails_after_invoke() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAuthorizer::new();
    let run_state = FailCompleteRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "authorized"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_resumes_approved_invocation_and_consumes_matching_lease() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context: context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    let consumed = leases.get(&scope, lease.grant.id).await.unwrap();
    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
}

#[tokio::test]
async fn capability_host_returns_dispatch_result_when_run_completion_fails_after_resume() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = FailCompleteRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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
    ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
}

#[tokio::test]
async fn capability_host_denies_resume_when_trust_ceiling_omits_capability_effect() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision_with_effects(Vec::new()),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::PolicyDenied,
            ..
        }
    ));
    assert!(!dispatcher.has_request());
    let active = leases.get(&scope, lease.grant.id).await.unwrap();
    assert_eq!(active.status, CapabilityLeaseStatus::Active);
}

#[tokio::test]
async fn capability_host_returns_dispatch_result_when_lease_consume_fails_after_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = ConsumeFailingLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context: context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    let claimed = leases.get(&scope, lease.grant.id).await.unwrap();
    assert_eq!(claimed.status, CapabilityLeaseStatus::Claimed);
}

#[tokio::test]
async fn capability_host_does_not_overwrite_completed_run_when_concurrent_resume_loses_claim() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let complete_notify = Arc::new(tokio::sync::Notify::new());
    let run_state = CompleteNotifyingRunStateStore::new(complete_notify.clone());
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = CoordinatedClaimConflictLeaseStore::new(complete_notify);
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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
    ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let first = resume_host.resume_json(CapabilityResumeRequest {
        context: context.clone(),
        approval_request_id: approval_id,
        capability_id: capability_id(),
        estimate: estimate.clone(),
        input: input.clone(),
        trust_decision: trust_decision(),
    });
    let second = resume_host.resume_json(CapabilityResumeRequest {
        context,
        approval_request_id: approval_id,
        capability_id: capability_id(),
        estimate,
        input,
        trust_decision: trust_decision(),
    });
    let (first_result, second_result) = tokio::join!(first, second);

    assert!(first_result.is_ok());
    assert!(matches!(
        second_result,
        Err(CapabilityInvocationError::Lease(_))
    ));
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
}

#[tokio::test]
async fn capability_host_rejects_resume_with_mismatched_capability_id() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let wrong_capability = CapabilityId::new("echo.other").unwrap();
    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: wrong_capability.clone(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    let message = err.to_string();
    assert!(!message.contains(capability_id().as_str()));
    match err {
        CapabilityInvocationError::ResumeContextMismatch { capability, kind } => {
            assert_eq!(capability, wrong_capability);
            assert_eq!(kind, ResumeContextMismatchKind::CapabilityId);
        }
        other => panic!("expected ResumeContextMismatch, got {other:?}"),
    }
    assert!(!dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ResumeContextMismatch"));
}

#[tokio::test]
async fn capability_host_rejects_resume_with_mismatched_approval_request_id() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();
    let real_approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    let bogus_approval_id = ApprovalRequestId::new();
    assert_ne!(bogus_approval_id, real_approval_id);

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: bogus_approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    let message = err.to_string();
    assert!(!message.contains(&real_approval_id.to_string()));
    assert!(!message.contains(&bogus_approval_id.to_string()));
    match err {
        CapabilityInvocationError::ResumeContextMismatch { capability, kind } => {
            assert_eq!(capability, capability_id());
            assert_eq!(kind, ResumeContextMismatchKind::ApprovalRequestId);
        }
        other => panic!("expected ResumeContextMismatch, got {other:?}"),
    }
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_rejects_resume_with_mutated_input_before_lease_claim_or_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: json!({"message": "approved"}),
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
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input: json!({"message": "mutated"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
    ));
    assert!(!dispatcher.has_request());
    let active = leases.get(&scope, lease.grant.id).await.unwrap();
    assert_eq!(active.status, CapabilityLeaseStatus::Active);
}

struct FailCompleteRunStateStore {
    inner: InMemoryRunStateStore,
}

impl FailCompleteRunStateStore {
    fn new() -> Self {
        Self {
            inner: InMemoryRunStateStore::new(),
        }
    }
}

#[async_trait]
impl RunStateStore for FailCompleteRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        Err(RunStateError::Filesystem(
            "complete transition unavailable".to_string(),
        ))
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

struct FailBlockApprovalRunStateStore {
    inner: InMemoryRunStateStore,
}

impl FailBlockApprovalRunStateStore {
    fn new() -> Self {
        Self {
            inner: InMemoryRunStateStore::new(),
        }
    }
}

#[async_trait]
impl RunStateStore for FailBlockApprovalRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        Err(RunStateError::Filesystem(
            "block approval unavailable".to_string(),
        ))
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

struct CompleteNotifyingRunStateStore {
    inner: InMemoryRunStateStore,
    complete_notify: Arc<tokio::sync::Notify>,
}

impl CompleteNotifyingRunStateStore {
    fn new(complete_notify: Arc<tokio::sync::Notify>) -> Self {
        Self {
            inner: InMemoryRunStateStore::new(),
            complete_notify,
        }
    }
}

#[async_trait]
impl RunStateStore for CompleteNotifyingRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        let record = self.inner.complete(scope, invocation_id).await?;
        self.complete_notify.notify_waiters();
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

struct FailOnFailRunStateStore {
    inner: InMemoryRunStateStore,
}

impl FailOnFailRunStateStore {
    fn new() -> Self {
        Self {
            inner: InMemoryRunStateStore::new(),
        }
    }
}

#[async_trait]
impl RunStateStore for FailOnFailRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.complete(scope, invocation_id).await
    }

    async fn fail(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        Err(RunStateError::Filesystem(
            "fail transition unavailable".to_string(),
        ))
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

struct CoordinatedClaimConflictLeaseStore {
    inner: InMemoryCapabilityLeaseStore,
    claim_calls: AtomicUsize,
    second_claim_started: tokio::sync::Notify,
    run_completed: Arc<tokio::sync::Notify>,
}

impl CoordinatedClaimConflictLeaseStore {
    fn new(run_completed: Arc<tokio::sync::Notify>) -> Self {
        Self {
            inner: InMemoryCapabilityLeaseStore::new(),
            claim_calls: AtomicUsize::new(0),
            second_claim_started: tokio::sync::Notify::new(),
            run_completed,
        }
    }
}

#[async_trait]
impl CapabilityLeaseStore for CoordinatedClaimConflictLeaseStore {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
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
        let call = self.claim_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            self.second_claim_started.notified().await;
            self.inner
                .claim(scope, lease_id, invocation_fingerprint)
                .await
        } else {
            self.second_claim_started.notify_waiters();
            self.run_completed.notified().await;
            Err(CapabilityLeaseError::InactiveLease {
                lease_id,
                status: CapabilityLeaseStatus::Consumed,
            })
        }
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.inner.consume(scope, lease_id).await
    }

    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        self.inner.leases_for_scope(scope).await
    }

    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.inner.active_leases_for_context(context).await
    }
}

struct ConsumeFailingLeaseStore {
    inner: InMemoryCapabilityLeaseStore,
}

impl ConsumeFailingLeaseStore {
    fn new() -> Self {
        Self {
            inner: InMemoryCapabilityLeaseStore::new(),
        }
    }
}

#[async_trait]
impl CapabilityLeaseStore for ConsumeFailingLeaseStore {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
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
        _scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        Err(CapabilityLeaseError::Persistence {
            reason: format!("consume failed for {lease_id}"),
        })
    }

    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        self.inner.leases_for_scope(scope).await
    }

    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.inner.active_leases_for_context(context).await
    }
}
