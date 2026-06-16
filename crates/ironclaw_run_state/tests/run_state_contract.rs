use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_filesystem::{
    DirEntry, FileStat, FilesystemError, FilesystemOperation, LocalFilesystem, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::*;
use ironclaw_run_state::*;

#[tokio::test]
async fn in_memory_run_state_tracks_running_to_completed() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

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

    let completed = store.complete(&scope, invocation_id).await.unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(
        store
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn in_memory_run_state_tracks_blocked_approval_with_request_id() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    let approval = approval_request(invocation_id);

    let blocked = store
        .block_approval(&scope, invocation_id, approval.clone())
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
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();

    let failed = store
        .fail(&scope, invocation_id, "AuthorizationDenied".to_string())
        .await
        .unwrap();

    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.error_kind.as_deref(), Some("AuthorizationDenied"));
}

#[tokio::test]
async fn run_state_transitions_fail_for_unknown_invocation() {
    let store = InMemoryRunStateStore::new();
    let missing = InvocationId::new();
    let scope = sample_scope(missing, "tenant1", "user1");

    let err = store.complete(&scope, missing).await.unwrap_err();

    assert!(
        matches!(err, RunStateError::UnknownInvocation { invocation_id } if invocation_id == missing)
    );
}

#[tokio::test]
async fn in_memory_run_state_rejects_duplicate_invocation_in_same_tenant_user() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.one").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    let err = store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.two").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        RunStateError::InvocationAlreadyExists { invocation_id: id } if id == invocation_id
    ));
    assert_eq!(
        store
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .capability_id,
        CapabilityId::new("echo.one").unwrap()
    );
}

#[tokio::test]
async fn filesystem_run_state_rejects_duplicate_invocation_in_same_tenant_user() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemRunStateStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.one").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    let err = store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.two").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        RunStateError::InvocationAlreadyExists { invocation_id: id } if id == invocation_id
    ));
    assert_eq!(
        store
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .capability_id,
        CapabilityId::new("echo.one").unwrap()
    );
}

#[tokio::test]
async fn filesystem_run_state_duplicate_start_is_serialized_across_store_instances() {
    let fs = Arc::new(ConcurrentMissingReadFilesystem::new(engine_filesystem()));
    let scoped = scoped_run_state_fs(fs);
    let first_store = FilesystemRunStateStore::new(Arc::clone(&scoped));
    let second_store = FilesystemRunStateStore::new(scoped);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");

    let (first, second) = tokio::join!(
        first_store.start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.one").unwrap(),
            scope: scope.clone(),
        }),
        second_store.start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.two").unwrap(),
            scope: scope.clone(),
        })
    );

    assert_eq!(
        [&first, &second]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1,
        "only one filesystem-backed store instance may create a given invocation"
    );
    assert_eq!(
        [&first, &second]
            .into_iter()
            .filter(|result| matches!(result, Err(RunStateError::InvocationAlreadyExists { invocation_id: id }) if *id == invocation_id))
            .count(),
        1,
        "the losing store instance should observe the record created by the winner"
    );
    assert!(
        first_store
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn in_memory_run_state_allows_same_invocation_id_in_different_tenants() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.one").unwrap(),
            scope: tenant_a.clone(),
        })
        .await
        .unwrap();
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.two").unwrap(),
            scope: tenant_b.clone(),
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .get(&tenant_a, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .capability_id,
        CapabilityId::new("echo.one").unwrap()
    );
    assert_eq!(
        store
            .get(&tenant_b, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .capability_id,
        CapabilityId::new("echo.two").unwrap()
    );
}

#[tokio::test]
async fn in_memory_run_state_hides_records_from_other_tenants_and_users() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let user_b = sample_scope(invocation_id, "tenant1", "user2");

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: tenant_a.clone(),
        })
        .await
        .unwrap();

    assert!(store.get(&tenant_b, invocation_id).await.unwrap().is_none());
    assert!(store.get(&user_b, invocation_id).await.unwrap().is_none());
    assert_eq!(
        store.records_for_scope(&tenant_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&user_b).await.unwrap(), Vec::new());
    assert!(matches!(
        store.complete(&tenant_b, invocation_id).await.unwrap_err(),
        RunStateError::UnknownInvocation { .. }
    ));
}

#[tokio::test]
async fn filesystem_run_state_store_persists_records_under_run_state_alias() {
    let fs = Arc::new(engine_filesystem());
    let scoped = scoped_run_state_fs(fs);
    let store = FilesystemRunStateStore::new(Arc::clone(&scoped));
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    store
        .block_approval(&scope, invocation_id, approval.clone())
        .await
        .unwrap();

    let reloaded = FilesystemRunStateStore::new(Arc::clone(&scoped))
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reloaded.status, RunStatus::BlockedApproval);
    assert_eq!(reloaded.approval_request_id, Some(approval.id));
    assert_eq!(
        FilesystemRunStateStore::new(scoped)
            .records_for_scope(&scope)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn filesystem_run_state_store_hides_records_from_other_tenants_and_users() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemRunStateStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let user_b = sample_scope(invocation_id, "tenant1", "user2");

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: tenant_a.clone(),
        })
        .await
        .unwrap();

    assert!(store.get(&tenant_b, invocation_id).await.unwrap().is_none());
    assert!(store.get(&user_b, invocation_id).await.unwrap().is_none());
    assert_eq!(
        store.records_for_scope(&tenant_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&user_b).await.unwrap(), Vec::new());
    assert!(matches!(
        store.complete(&tenant_b, invocation_id).await.unwrap_err(),
        RunStateError::UnknownInvocation { .. }
    ));
}

#[tokio::test]
async fn filesystem_approval_request_store_persists_pending_requests_under_approvals_alias() {
    let fs = Arc::new(engine_filesystem());
    let scoped = scoped_run_state_fs(fs);
    let store = FilesystemApprovalRequestStore::new(Arc::clone(&scoped));
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);

    let record = store
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();

    assert_eq!(record.scope, scope);
    assert_eq!(record.status, ApprovalStatus::Pending);
    assert_eq!(record.request, approval);
    let reloaded = FilesystemApprovalRequestStore::new(scoped)
        .get(&record.scope, record.request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded, record);
}

#[tokio::test]
async fn filesystem_approval_request_duplicate_save_is_serialized_across_store_instances() {
    let fs = Arc::new(ConcurrentMissingReadFilesystem::new(engine_filesystem()));
    let scoped = scoped_run_state_fs(fs);
    let first_store = FilesystemApprovalRequestStore::new(Arc::clone(&scoped));
    let second_store = FilesystemApprovalRequestStore::new(scoped);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);

    let (first, second) = tokio::join!(
        first_store.save_pending(scope.clone(), approval.clone()),
        second_store.save_pending(scope.clone(), approval.clone())
    );

    assert_eq!(
        [&first, &second]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1,
        "only one filesystem-backed store instance may create a given approval request"
    );
    assert_eq!(
        [&first, &second]
            .into_iter()
            .filter(|result| matches!(result, Err(RunStateError::ApprovalRequestAlreadyExists { request_id }) if *request_id == approval.id))
            .count(),
        1,
        "the losing approval store instance should observe the winner's pending request"
    );
    assert!(
        first_store
            .get(&scope, approval.id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn filesystem_approval_request_listing_ignores_records_deleted_after_list() {
    let fs = Arc::new(DisappearingApprovalReadFilesystem::new(engine_filesystem()));
    let store = FilesystemApprovalRequestStore::new(scoped_run_state_fs(Arc::clone(&fs)));
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);

    store.save_pending(scope.clone(), approval).await.unwrap();
    fs.fail_next_approval_read();

    let records = store.records_for_scope(&scope).await.unwrap();

    assert_eq!(records, Vec::new());
}

#[tokio::test]
async fn in_memory_approval_request_store_discards_pending_request() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    let saved = store.save_pending(scope.clone(), approval).await.unwrap();
    let discarded = store.discard_pending(&scope, request_id).await.unwrap();

    assert_eq!(discarded, saved);
    assert!(store.get(&scope, request_id).await.unwrap().is_none());
    assert_eq!(store.records_for_scope(&scope).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn filesystem_approval_request_store_discards_pending_request() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemApprovalRequestStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    let saved = store.save_pending(scope.clone(), approval).await.unwrap();
    let discarded = store.discard_pending(&scope, request_id).await.unwrap();

    assert_eq!(discarded, saved);
    assert!(store.get(&scope, request_id).await.unwrap().is_none());
    assert_eq!(store.records_for_scope(&scope).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn in_memory_approval_store_allows_same_request_id_in_different_tenants() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let approval = approval_request(invocation_id);

    store
        .save_pending(tenant_a.clone(), approval.clone())
        .await
        .unwrap();
    store
        .save_pending(tenant_b.clone(), approval.clone())
        .await
        .unwrap();

    assert_eq!(
        store
            .get(&tenant_a, approval.id)
            .await
            .unwrap()
            .unwrap()
            .scope,
        tenant_a
    );
    assert_eq!(
        store
            .get(&tenant_b, approval.id)
            .await
            .unwrap()
            .unwrap()
            .scope,
        tenant_b
    );
}

#[tokio::test]
async fn approval_request_store_hides_records_from_other_tenants_and_users() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemApprovalRequestStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let user_b = sample_scope(invocation_id, "tenant1", "user2");
    let approval = approval_request(invocation_id);

    let record = store.save_pending(tenant_a, approval).await.unwrap();

    assert!(
        store
            .get(&tenant_b, record.request.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get(&user_b, record.request.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.records_for_scope(&tenant_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&user_b).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn run_state_isolates_records_by_agent_scope() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let agent_a = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-a"));
    let agent_b = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-b"));

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: agent_a.clone(),
        })
        .await
        .unwrap();

    assert!(store.get(&agent_b, invocation_id).await.unwrap().is_none());
    assert_eq!(store.records_for_scope(&agent_b).await.unwrap(), Vec::new());
    assert!(matches!(
        store.complete(&agent_b, invocation_id).await.unwrap_err(),
        RunStateError::UnknownInvocation { .. }
    ));
}

#[tokio::test]
async fn filesystem_run_state_uses_agent_scoped_paths() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemRunStateStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let agent_a = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-a"));
    let agent_b = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-b"));

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: agent_a.clone(),
        })
        .await
        .unwrap();

    assert!(store.get(&agent_b, invocation_id).await.unwrap().is_none());
    assert_eq!(store.records_for_scope(&agent_b).await.unwrap(), Vec::new());
    assert_eq!(store.records_for_scope(&agent_a).await.unwrap().len(), 1);
}

#[tokio::test]
async fn approval_request_store_isolates_records_by_agent_scope() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let agent_a = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-a"));
    let agent_b = sample_scope_with_agent(invocation_id, "tenant1", "user1", Some("agent-b"));
    let approval = approval_request(invocation_id);

    let record = store.save_pending(agent_a.clone(), approval).await.unwrap();

    assert!(
        store
            .get(&agent_b, record.request.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.records_for_scope(&agent_b).await.unwrap(), Vec::new());
    assert_eq!(store.records_for_scope(&agent_a).await.unwrap().len(), 1);
}

#[tokio::test]
async fn run_state_isolates_records_by_project_scope() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let project_a = sample_scope(invocation_id, "tenant1", "user1");
    let mut project_b = project_a.clone();
    project_b.project_id = Some(ProjectId::new("project2").unwrap());

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: project_a.clone(),
        })
        .await
        .unwrap();

    assert!(
        store
            .get(&project_b, invocation_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.records_for_scope(&project_b).await.unwrap(),
        Vec::new()
    );
    assert!(matches!(
        store.complete(&project_b, invocation_id).await.unwrap_err(),
        RunStateError::UnknownInvocation { .. }
    ));
}

#[tokio::test]
async fn filesystem_run_state_isolates_records_by_project_scope() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemRunStateStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let project_a = sample_scope(invocation_id, "tenant1", "user1");
    let mut project_b = project_a.clone();
    project_b.project_id = Some(ProjectId::new("project2").unwrap());

    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: project_a.clone(),
        })
        .await
        .unwrap();

    assert!(
        store
            .get(&project_b, invocation_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.records_for_scope(&project_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&project_a).await.unwrap().len(), 1);
}

#[tokio::test]
async fn run_state_clears_stale_approval_request_on_non_approval_transitions() {
    let store = InMemoryRunStateStore::new();
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    store
        .block_approval(&scope, invocation_id, approval_request(invocation_id))
        .await
        .unwrap();

    let auth_blocked = store
        .block_auth(&scope, invocation_id, "ExternalAuth".to_string())
        .await
        .unwrap();
    assert_eq!(auth_blocked.approval_request_id, None);

    store
        .block_approval(&scope, invocation_id, approval_request(invocation_id))
        .await
        .unwrap();
    let failed = store
        .fail(&scope, invocation_id, "AuthorizationDenied".to_string())
        .await
        .unwrap();
    assert_eq!(failed.approval_request_id, None);

    store
        .block_approval(&scope, invocation_id, approval_request(invocation_id))
        .await
        .unwrap();
    let completed = store.complete(&scope, invocation_id).await.unwrap();
    assert_eq!(completed.approval_request_id, None);
}

#[tokio::test]
async fn approval_request_store_isolates_records_by_project_scope() {
    let store = InMemoryApprovalRequestStore::new();
    let invocation_id = InvocationId::new();
    let project_a = sample_scope(invocation_id, "tenant1", "user1");
    let mut project_b = project_a.clone();
    project_b.project_id = Some(ProjectId::new("project2").unwrap());
    let approval = approval_request(invocation_id);

    let record = store
        .save_pending(project_a.clone(), approval)
        .await
        .unwrap();

    assert!(
        store
            .get(&project_b, record.request.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.records_for_scope(&project_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&project_a).await.unwrap().len(), 1);
}

#[tokio::test]
async fn filesystem_approval_request_store_isolates_records_by_project_scope() {
    let fs = Arc::new(engine_filesystem());
    let store = FilesystemApprovalRequestStore::new(scoped_run_state_fs(fs));
    let invocation_id = InvocationId::new();
    let project_a = sample_scope(invocation_id, "tenant1", "user1");
    let mut project_b = project_a.clone();
    project_b.project_id = Some(ProjectId::new("project2").unwrap());
    let approval = approval_request(invocation_id);

    let record = store
        .save_pending(project_a.clone(), approval)
        .await
        .unwrap();

    assert!(
        store
            .get(&project_b, record.request.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.records_for_scope(&project_b).await.unwrap(),
        Vec::new()
    );
    assert_eq!(store.records_for_scope(&project_a).await.unwrap().len(), 1);
}

/// Regression for the ScopedFilesystem migration: two stores share one
/// underlying [`RootFilesystem`] but each is constructed with a
/// [`MountView`] whose `/run-state` and `/approvals` aliases resolve to a
/// different tenant-scoped [`VirtualPath`] subtree. Writing the same
/// `(user_id, project_id, invocation_id)` tuple on tenant A's store must
/// NOT make the record visible from tenant B's store. Before this
/// migration, the filesystem run-state store held a raw `&F: RootFilesystem`
/// and encoded tenant identity in the path itself — any composition layer
/// that forgot to prefix the path with tenant would leak across tenants,
/// with the type system saying nothing. The structural fix routes every op
/// through `ScopedFilesystem`, so two MountViews over the same backend
/// cannot see each other's data.
#[tokio::test]
async fn filesystem_run_state_store_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(engine_filesystem());
    let scoped_a = scoped_run_state_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let scoped_b = scoped_run_state_fs_at(Arc::clone(&backend), "tenant-b", "alice");

    let runs_a = FilesystemRunStateStore::new(Arc::clone(&scoped_a));
    let runs_b = FilesystemRunStateStore::new(Arc::clone(&scoped_b));
    let approvals_a = FilesystemApprovalRequestStore::new(scoped_a);
    let approvals_b = FilesystemApprovalRequestStore::new(scoped_b);

    // Identical `(user_id, project_id, invocation_id)` for both — the only
    // thing keeping the two stores apart is the mount-time tenant prefix.
    let invocation_id = InvocationId::new();
    let scope_a = ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("alice").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project-1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let scope_b = ResourceScope {
        tenant_id: TenantId::new("tenant-b").unwrap(),
        ..scope_a.clone()
    };
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    runs_a
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope_a.clone(),
        })
        .await
        .unwrap();
    approvals_a
        .save_pending(scope_a.clone(), approval)
        .await
        .unwrap();

    // Tenant A sees its own run and approval.
    assert!(
        runs_a.get(&scope_a, invocation_id).await.unwrap().is_some(),
        "tenant A must see the run it just wrote"
    );
    assert!(
        approvals_a
            .get(&scope_a, request_id)
            .await
            .unwrap()
            .is_some(),
        "tenant A must see the approval it just wrote"
    );

    // Tenant B's stores do NOT see tenant A's records, despite identical
    // (user_id, project_id, invocation_id, request_id). Both `get` and
    // `records_for_scope` must fail closed; transitions targeted at
    // tenant B's view of the same id must report unknown.
    assert!(
        runs_b.get(&scope_b, invocation_id).await.unwrap().is_none(),
        "tenant B must NOT see tenant A's run (cross-tenant path leak)"
    );
    assert!(
        approvals_b
            .get(&scope_b, request_id)
            .await
            .unwrap()
            .is_none(),
        "tenant B must NOT see tenant A's approval (cross-tenant path leak)"
    );
    assert!(
        runs_b.records_for_scope(&scope_b).await.unwrap().is_empty(),
        "tenant B records_for_scope must be empty under shared (user, project)"
    );
    assert!(
        approvals_b
            .records_for_scope(&scope_b)
            .await
            .unwrap()
            .is_empty(),
        "tenant B approvals records_for_scope must be empty under shared (user, project)"
    );
    assert!(matches!(
        runs_b.complete(&scope_b, invocation_id).await.unwrap_err(),
        RunStateError::UnknownInvocation { .. }
    ));
    assert!(matches!(
        approvals_b.approve(&scope_b, request_id).await.unwrap_err(),
        RunStateError::UnknownApprovalRequest { .. }
    ));
}

struct ConcurrentMissingReadFilesystem {
    inner: LocalFilesystem,
    missing_reads: AtomicUsize,
}

impl ConcurrentMissingReadFilesystem {
    fn new(inner: LocalFilesystem) -> Self {
        Self {
            inner,
            missing_reads: AtomicUsize::new(0),
        }
    }

    fn should_race_missing_read(path: &VirtualPath) -> bool {
        path.as_str().starts_with("/engine/") && path.as_str().ends_with(".json")
    }
}

#[async_trait]
impl RootFilesystem for ConcurrentMissingReadFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        match self.inner.read_file(path).await {
            Ok(bytes) => Ok(bytes),
            Err(error)
                if matches!(error, FilesystemError::NotFound { .. })
                    && Self::should_race_missing_read(path) =>
            {
                self.missing_reads.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(25));
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.append_file(path, bytes).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }

    // After PR #3659 the consumer's existence-check / read path goes
    // through `get`, not `read_file`. Mirror the missing-read race
    // behavior here so the duplicate-start serialization test still
    // exercises the concurrent-create window it was designed for.
    async fn put(
        &self,
        path: &VirtualPath,
        entry: ironclaw_filesystem::Entry,
        cas: ironclaw_filesystem::CasExpectation,
    ) -> Result<ironclaw_filesystem::RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, FilesystemError> {
        let result = self.inner.get(path).await;
        if matches!(result, Ok(None)) && Self::should_race_missing_read(path) {
            self.missing_reads.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(25));
        }
        result
    }
}

struct DisappearingApprovalReadFilesystem {
    inner: LocalFilesystem,
    fail_next_approval_read: AtomicBool,
}

impl DisappearingApprovalReadFilesystem {
    fn new(inner: LocalFilesystem) -> Self {
        Self {
            inner,
            fail_next_approval_read: AtomicBool::new(false),
        }
    }

    fn fail_next_approval_read(&self) {
        self.fail_next_approval_read.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl RootFilesystem for DisappearingApprovalReadFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        if path.as_str().contains("/approvals/")
            && path.as_str().ends_with(".json")
            && self.fail_next_approval_read.swap(false, Ordering::SeqCst)
        {
            return Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            });
        }
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.append_file(path, bytes).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }

    // After PR #3659 the approval-listing read path goes through `get`,
    // not `read_file`. Mirror the fault injection so the
    // disappearing-approval test still exercises its intended path.
    async fn put(
        &self,
        path: &VirtualPath,
        entry: ironclaw_filesystem::Entry,
        cas: ironclaw_filesystem::CasExpectation,
    ) -> Result<ironclaw_filesystem::RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, FilesystemError> {
        if path.as_str().contains("/approvals/")
            && path.as_str().ends_with(".json")
            && self.fail_next_approval_read.swap(false, Ordering::SeqCst)
        {
            return Ok(None);
        }
        self.inner.get(path).await
    }
}

/// Build a [`LocalFilesystem`] with `/engine` mounted to a tempdir. The
/// `/run-state` and `/approvals` mount aliases on the outer
/// [`ScopedFilesystem`] resolve under `/engine/...` so the legacy local-disk
/// fault-injection wrappers (which match by post-resolution path) keep
/// working unchanged.
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

/// Wrap a [`RootFilesystem`] in a [`ScopedFilesystem`] that exposes
/// `/run-state` and `/approvals` aliases, both rooted under a single
/// tenant/user subtree of the underlying mount. Tests share one
/// `MountView` between the run-state and approval stores so a single
/// composition can drive both surfaces over the same backend (the
/// production composition shape).
fn scoped_run_state_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    scoped_run_state_fs_at(backend, "test-tenant", "test-user")
}

/// Variant of [`scoped_run_state_fs`] that resolves the `/run-state` and
/// `/approvals` aliases under a caller-chosen tenant/user prefix. Used by
/// the cross-tenant isolation regression test to materialize two
/// `ScopedFilesystem`s with disjoint `MountView` targets over the same
/// `RootFilesystem`.
fn scoped_run_state_fs_at<F>(backend: Arc<F>, tenant: &str, user: &str) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let tenant_user_prefix = format!("/engine/tenants/{tenant}/users/{user}");
    let mounts = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/run-state").expect("alias"),
            VirtualPath::new(format!("{tenant_user_prefix}/run-state")).expect("target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/approvals").expect("alias"),
            VirtualPath::new(format!("{tenant_user_prefix}/approvals")).expect("target"),
            MountPermissions::read_write_list_delete(),
        ),
    ])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
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

fn sample_scope_with_agent(
    invocation_id: InvocationId,
    tenant: &str,
    user: &str,
    agent: Option<&str>,
) -> ResourceScope {
    let mut scope = sample_scope(invocation_id, tenant, user);
    scope.agent_id = agent.map(|id| AgentId::new(id).unwrap());
    scope
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
