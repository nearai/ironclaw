use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_run_state::*;

#[tokio::test]
async fn in_memory_run_state_tracks_running_to_completed() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let scope = sample_scope(invocation_id);

    let running = store
        .start(RunStart {
            invocation_id,
            capability_id: capability_id.clone(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    assert_eq!(running.status, RunStatus::Running);
    assert_eq!(running.capability_id, capability_id);
    assert_eq!(running.scope, scope);

    let completed = store.complete(invocation_id).await.unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(
        store.get(invocation_id).await.unwrap().unwrap().status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn in_memory_run_state_tracks_blocked_approval_with_request_id() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: sample_scope(invocation_id),
        })
        .await
        .unwrap();
    let approval = approval_request(invocation_id);

    let blocked = store
        .block_approval(invocation_id, approval.clone())
        .await
        .unwrap();

    assert_eq!(blocked.status, RunStatus::BlockedApproval);
    assert_eq!(blocked.approval_request_id, Some(approval.id));
    assert_eq!(blocked.error_kind, None);
}

#[tokio::test]
async fn in_memory_run_state_tracks_failed_with_error_kind() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: sample_scope(invocation_id),
        })
        .await
        .unwrap();

    let failed = store
        .fail(invocation_id, "AuthorizationDenied".to_string())
        .await
        .unwrap();

    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.error_kind.as_deref(), Some("AuthorizationDenied"));
}

#[tokio::test]
async fn run_state_transitions_fail_for_unknown_invocation() {
    let store = InMemoryRunStateStore::new();
    let missing = InvocationId::new();

    let err = store.complete(missing).await.unwrap_err();

    assert!(
        matches!(err, RunStateError::UnknownInvocation { invocation_id } if invocation_id == missing)
    );
}

#[tokio::test]
async fn filesystem_run_state_store_persists_records_under_engine_runs() {
    let fs = engine_filesystem();
    let store = FilesystemRunStateStore::new(&fs);
    let invocation_id = InvocationId::new();
    let approval = approval_request(invocation_id);

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: sample_scope(invocation_id),
        })
        .await
        .unwrap();
    store
        .block_approval(invocation_id, approval.clone())
        .await
        .unwrap();

    let reloaded = FilesystemRunStateStore::new(&fs)
        .get(invocation_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reloaded.status, RunStatus::BlockedApproval);
    assert_eq!(reloaded.approval_request_id, Some(approval.id));
    assert_eq!(
        FilesystemRunStateStore::new(&fs)
            .records()
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn filesystem_approval_request_store_persists_pending_requests_under_engine_approvals() {
    let fs = engine_filesystem();
    let store = FilesystemApprovalRequestStore::new(&fs);
    let invocation_id = InvocationId::new();
    let approval = approval_request(invocation_id);

    let record = store.save_pending(approval.clone()).await.unwrap();

    assert_eq!(record.status, ApprovalStatus::Pending);
    assert_eq!(record.request, approval);
    let reloaded = FilesystemApprovalRequestStore::new(&fs)
        .get(record.request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded, record);
}

fn engine_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
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
