use ironclaw_host_api::*;
use ironclaw_run_state::*;

#[tokio::test]
async fn approval_store_marks_pending_request_approved_or_denied_with_scope() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    store.save_pending(scope.clone(), approval).await.unwrap();
    let approved = store.approve(&scope, request_id).await.unwrap();

    assert_eq!(approved.status, ApprovalStatus::Approved);
    assert_eq!(
        store.get(&scope, request_id).await.unwrap().unwrap().status,
        ApprovalStatus::Approved
    );

    let denied_request = approval_request(invocation_id);
    let denied_id = denied_request.id;
    store
        .save_pending(scope.clone(), denied_request)
        .await
        .unwrap();
    let denied = store.deny(&scope, denied_id).await.unwrap();

    assert_eq!(denied.status, ApprovalStatus::Denied);
}

#[tokio::test]
async fn approval_resolution_is_scoped_to_tenant_and_user() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    store.save_pending(tenant_a, approval).await.unwrap();

    let err = store.approve(&tenant_b, request_id).await.unwrap_err();

    assert!(matches!(
        err,
        RunStateError::UnknownApprovalRequest { request_id: id } if id == request_id
    ));
}

#[tokio::test]
async fn approval_store_rejects_second_resolution_attempt() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    store.save_pending(scope.clone(), approval).await.unwrap();
    store.approve(&scope, request_id).await.unwrap();

    let err = store.deny(&scope, request_id).await.unwrap_err();

    assert!(matches!(
        err,
        RunStateError::ApprovalNotPending {
            request_id: id,
            status: ApprovalStatus::Approved,
        } if id == request_id
    ));
}

#[tokio::test]
async fn approval_store_rejects_duplicate_pending_save() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    store
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    store.approve(&scope, request_id).await.unwrap();

    let err = store
        .save_pending(scope.clone(), approval)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        RunStateError::ApprovalRequestAlreadyExists { request_id: id } if id == request_id
    ));
    assert_eq!(
        store.get(&scope, request_id).await.unwrap().unwrap().status,
        ApprovalStatus::Approved
    );
}

#[tokio::test]
async fn filesystem_approval_store_rejects_second_resolution_attempt() {
    let fs = engine_filesystem();
    let store = FilesystemApprovalRequestStore::new(&fs);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    store.save_pending(scope.clone(), approval).await.unwrap();
    store.approve(&scope, request_id).await.unwrap();

    let err = store.deny(&scope, request_id).await.unwrap_err();

    assert!(matches!(
        err,
        RunStateError::ApprovalNotPending {
            request_id: id,
            status: ApprovalStatus::Approved,
        } if id == request_id
    ));
}

#[tokio::test]
async fn filesystem_approval_store_rejects_duplicate_pending_save() {
    let fs = engine_filesystem();
    let store = FilesystemApprovalRequestStore::new(&fs);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    store
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    store.approve(&scope, request_id).await.unwrap();

    let err = store
        .save_pending(scope.clone(), approval)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        RunStateError::ApprovalRequestAlreadyExists { request_id: id } if id == request_id
    ));
    assert_eq!(
        store.get(&scope, request_id).await.unwrap().unwrap().status,
        ApprovalStatus::Approved
    );
}

fn engine_filesystem() -> ironclaw_filesystem::LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = ironclaw_filesystem::LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn sample_scope(invocation_id: InvocationId, tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
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
        invocation_fingerprint: None,
        reason: format!("approval for {invocation_id}"),
        reusable_scope: None,
    }
}
