use ironclaw_host_api::*;
use ironclaw_run_state::*;

#[test]
fn in_memory_run_state_tracks_running_to_completed() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let scope = sample_scope(invocation_id);

    let running = store.start(RunStart {
        invocation_id,
        capability_id: capability_id.clone(),
        scope: scope.clone(),
    });
    assert_eq!(running.status, RunStatus::Running);
    assert_eq!(running.capability_id, capability_id);
    assert_eq!(running.scope, scope);

    let completed = store.complete(invocation_id).unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(
        store.get(invocation_id).unwrap().status,
        RunStatus::Completed
    );
}

#[test]
fn in_memory_run_state_tracks_blocked_approval_with_request_id() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    store.start(RunStart {
        invocation_id,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        scope: sample_scope(invocation_id),
    });
    let approval = approval_request(invocation_id);

    let blocked = store
        .block_approval(invocation_id, approval.clone())
        .unwrap();

    assert_eq!(blocked.status, RunStatus::BlockedApproval);
    assert_eq!(blocked.approval_request_id, Some(approval.id));
    assert_eq!(blocked.error_kind, None);
}

#[test]
fn in_memory_run_state_tracks_failed_with_error_kind() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    store.start(RunStart {
        invocation_id,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        scope: sample_scope(invocation_id),
    });

    let failed = store
        .fail(invocation_id, "AuthorizationDenied".to_string())
        .unwrap();

    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.error_kind.as_deref(), Some("AuthorizationDenied"));
}

#[test]
fn run_state_transitions_fail_for_unknown_invocation() {
    let store = InMemoryRunStateStore::new();
    let missing = InvocationId::new();

    let err = store.complete(missing).unwrap_err();

    assert!(
        matches!(err, RunStateError::UnknownInvocation { invocation_id } if invocation_id == missing)
    );
}

fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

fn approval_request(invocation_id: InvocationId) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
        action: Box::new(Action::Dispatch {
            capability: CapabilityId::new("echo.say").unwrap(),
            estimated_resources: ResourceEstimate::default(),
        }),
        reason: format!("approval for {invocation_id}"),
        reusable_scope: None,
    }
}
