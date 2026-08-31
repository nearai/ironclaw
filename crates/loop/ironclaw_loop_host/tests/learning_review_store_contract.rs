use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProjectId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
    turn::TurnRunId,
};
use ironclaw_loop_host::learning_review::FilesystemLearningCandidateStore;
use ironclaw_memory::{
    LearningCandidateInsert, LearningCandidateStore, LearningDecision, LearningExplicitness,
    LearningReview, LearningReviewRecord, LearningScope, MemoryLearningProposal,
    MemoryLearningProposalKind,
};

fn resource_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("user-a").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
    .tenant_shared_managed_scope()
}

fn learning_scope() -> LearningScope {
    LearningScope::new(
        TenantId::new("tenant-a").expect("tenant"),
        UserId::new("user-a").expect("user"),
        AgentId::new("agent-a").expect("agent"),
        Some(ProjectId::new("project-a").expect("project")),
    )
}

fn record_for_scope(run_id: TurnRunId, scope: LearningScope) -> LearningReviewRecord {
    LearningReviewRecord::new(
        run_id,
        scope,
        LearningReview {
            memory: vec![MemoryLearningProposal {
                kind: MemoryLearningProposalKind::Preference,
                content: "Use short status reports".to_string(),
                source_message_indices: vec![0],
                confidence_basis_points: 9_000,
                explicitness: LearningExplicitness::Explicit,
                tainted: false,
            }],
            skill: LearningDecision::skip(),
        },
    )
    .expect("record")
}

fn record(run_id: TurnRunId) -> LearningReviewRecord {
    record_for_scope(run_id, learning_scope())
}

#[tokio::test]
async fn candidate_store_is_idempotent_by_run() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem = ScopedFilesystem::new(backend, |scope| {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared")?,
            VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id.as_str()))?,
            MountPermissions::read_write(),
        )])
    });
    let store = FilesystemLearningCandidateStore::new(Arc::new(filesystem), resource_scope());
    let record = record(TurnRunId::new());

    assert_eq!(
        store.insert_if_absent(&record).await.expect("first insert"),
        LearningCandidateInsert::Created
    );
    assert_eq!(
        store
            .insert_if_absent(&record)
            .await
            .expect("replay insert"),
        LearningCandidateInsert::AlreadyExists
    );
    assert_eq!(
        store
            .get(&learning_scope(), record.run_id)
            .await
            .expect("get candidate"),
        Some(record.clone())
    );
    assert_eq!(
        store
            .list_unresolved(&learning_scope())
            .await
            .expect("list unresolved"),
        vec![record]
    );
}

#[tokio::test]
async fn candidate_store_isolates_tenants_with_same_user_agent_and_project() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem = Arc::new(ScopedFilesystem::new(backend, |scope| {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared")?,
            VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id.as_str()))?,
            MountPermissions::read_write(),
        )])
    }));
    let store_a = FilesystemLearningCandidateStore::new(Arc::clone(&filesystem), resource_scope());
    let tenant_b_resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant-b").expect("tenant"),
        user_id: UserId::new("user-a").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
    .tenant_shared_managed_scope();
    let store_b =
        FilesystemLearningCandidateStore::new(Arc::clone(&filesystem), tenant_b_resource_scope);
    let run_id = TurnRunId::new();
    let tenant_a_scope = learning_scope();
    let tenant_a = record_for_scope(run_id, tenant_a_scope.clone());
    let tenant_b_scope = LearningScope::new(
        TenantId::new("tenant-b").expect("tenant"),
        UserId::new("user-a").expect("user"),
        AgentId::new("agent-a").expect("agent"),
        Some(ProjectId::new("project-a").expect("project")),
    );
    let tenant_b = record_for_scope(run_id, tenant_b_scope.clone());
    assert_eq!(
        store_a.insert_if_absent(&tenant_a).await.expect("tenant a"),
        LearningCandidateInsert::Created
    );
    assert_eq!(
        store_b.insert_if_absent(&tenant_b).await.expect("tenant b"),
        LearningCandidateInsert::Created
    );
    assert_eq!(
        store_a.get(&tenant_a_scope, run_id).await.expect("read a"),
        Some(tenant_a.clone())
    );
    assert_eq!(
        store_b.get(&tenant_b_scope, run_id).await.expect("read b"),
        Some(tenant_b.clone())
    );
    assert_eq!(
        store_a
            .list_unresolved(&tenant_a_scope)
            .await
            .expect("list a"),
        vec![tenant_a]
    );
    assert_eq!(
        store_b
            .list_unresolved(&tenant_b_scope)
            .await
            .expect("list b"),
        vec![tenant_b]
    );
}
