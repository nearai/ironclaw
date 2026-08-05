use std::sync::Arc;

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::turn::{
    LoopResultRef, ReplyTargetBindingRef, SourceBindingRef, TurnGateRef, TurnRunId, TurnScope,
};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_loop_host::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
use ironclaw_processes::{
    CloseProcessDependencyRequest, ProcessDependencyPort, ProcessDependencyQuery,
    ProcessDependencyState, ProcessDependencySubmission, ProcessJournalStore, ProcessKind,
    ProcessLifecycleStatus, ProcessOperationId, ProcessSubmissionPort, ProcessTerminalEvidence,
    SettleProcessDependencyRequest, SubmitProcessRequest,
};
use ironclaw_runner::subagent::await_edge::{EdgeTerminalKind, store::AwaitEdgeStore};

#[tokio::test]
async fn runner_await_edge_is_a_projection_over_process_dependencies() {
    let filesystem = processes_filesystem();
    let journal = Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem)));
    let parent_scope = turn_scope("parent-thread");
    let child_scope = turn_scope("child-thread");
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();
    submit_root(
        &journal,
        &parent_scope.to_resource_scope(),
        ProcessId::from_uuid(parent_run_id.as_uuid()),
    )
    .await;

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context("parent");
    parent_context.run_id = parent_run_id;
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_scope.thread_id.clone();
    let dependencies = Arc::clone(&journal)
        as Arc<dyn ProcessDependencyPort<Error = ironclaw_processes::ProcessJournalStoreError>>;
    let store = AwaitEdgeStore::new(dependencies);
    let dependency_metadata = serde_json::to_value(AwaitedChildSetRecord {
        gate_ref: TurnGateRef::new("gate:child").expect("gate"),
        parent_run_context: parent_context,
        tree_root_run_id: parent_run_id,
        child_scope: child_scope.clone(),
        child_run_id,
        child_thread_id: child_scope.thread_id.clone(),
        source_binding_ref: SourceBindingRef::new("source:child").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:child").expect("reply target"),
        subagent_kind: SubagentKindId::new("general").expect("subagent kind"),
        spawn_capability_id: CapabilityId::new("builtin.subagent.spawn").expect("capability"),
        result_ref: LoopResultRef::new("result:child").expect("result ref"),
        mode: SpawnSubagentMode::Blocking,
    })
    .expect("dependency metadata");
    submit_child(
        &journal,
        &child_scope.to_resource_scope(),
        parent_run_id,
        child_run_id,
        1,
        Some(ProcessDependencySubmission {
            dependent_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
            root_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
            group_ref: Some("gate:child".to_string()),
            metadata: dependency_metadata,
        }),
    )
    .await;

    let settled = store
        .settle(
            &child_scope,
            parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(17),
            None,
        )
        .await
        .expect("settle edge")
        .expect("edge exists");
    assert_eq!(settled.terminal_kind, Some(EdgeTerminalKind::Completed));
    store
        .consume(&child_scope, parent_run_id, child_run_id)
        .await
        .expect("consume edge");
    assert!(
        store
            .peek(&child_scope, parent_run_id, child_run_id)
            .await
            .expect("peek consumed edge")
            .is_none()
    );
    assert!(
        journal
            .unresolved_process_dependencies()
            .await
            .expect("unresolved dependencies")
            .is_empty()
    );
}

#[tokio::test]
async fn process_dependency_journal_stress_closes_each_record_and_releases_capacity() {
    const CHILDREN: u32 = 96;

    let filesystem = processes_filesystem();
    let journal = Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem)));
    let parent_scope = resource_scope("stress-parent");
    let parent_id = ProcessId::new();
    submit_root(&journal, &parent_scope, parent_id).await;

    let mut children = Vec::with_capacity(CHILDREN as usize);
    for index in 0..CHILDREN {
        let child_id = ProcessId::new();
        let child_scope = resource_scope(&format!("stress-child-{index}"));
        journal
            .submit_process(SubmitProcessRequest {
                process_id: child_id,
                process_kind: ProcessKind::AgentTurn,
                scope: child_scope.clone(),
                exclusive_within_scope: false,
                operation_id: Some(ProcessOperationId::from_trusted(format!("child-{index}"))),
                owner_user_id: Some(child_scope.user_id.clone()),
                concurrency_class: None,
                parent_process_id: Some(parent_id),
                root_process_id: Some(parent_id),
                spawn_tree_descendant_cap: Some(CHILDREN),
                dependency: Some(ProcessDependencySubmission {
                    dependent_process_id: parent_id,
                    root_process_id: parent_id,
                    group_ref: Some("stress-group".to_string()),
                    metadata: serde_json::json!({"index": index}),
                }),
                checkpoint_ref: None,
                input: None,
                created_at: Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit child with dependency");
        children.push((child_id, child_scope));
    }

    let mut tasks = Vec::with_capacity(CHILDREN as usize);
    for (child_id, child_scope) in children {
        let filesystem = Arc::clone(&filesystem);
        tasks.push(tokio::spawn(async move {
            let handle = ProcessJournalStore::new(filesystem);
            handle
                .settle_process_dependency(SettleProcessDependencyRequest {
                    dependent_process_id: parent_id,
                    dependency_process_id: child_id,
                    scope: child_scope.clone(),
                    terminal: ProcessTerminalEvidence {
                        status: ProcessLifecycleStatus::Completed,
                        output_bytes: Some(1),
                        sanitized_reason: None,
                    },
                    settled_at: Utc::now(),
                })
                .await
                .expect("settle dependency");
            handle
                .consume_process_dependency(CloseProcessDependencyRequest {
                    dependent_process_id: parent_id,
                    dependency_process_id: child_id,
                    scope: child_scope,
                    closed_at: Utc::now(),
                })
                .await
                .expect("consume dependency");
        }));
    }
    for task in tasks {
        task.await.expect("dependency task");
    }

    assert!(
        journal
            .unresolved_process_dependencies()
            .await
            .expect("unresolved dependencies")
            .is_empty()
    );
    let closed = journal
        .query_process_dependencies(ProcessDependencyQuery {
            scope: parent_scope.clone(),
            dependent_process_id: Some(parent_id),
            group_ref: Some("stress-group".to_string()),
            include_closed: true,
        })
        .await
        .expect("closed dependency query");
    assert_eq!(closed.len(), CHILDREN as usize);
    assert!(
        closed
            .iter()
            .all(|record| record.state == ProcessDependencyState::Consumed)
    );

    submit_child(
        &journal,
        &resource_scope("stress-replacement"),
        TurnRunId::from_uuid(parent_id.as_uuid()),
        TurnRunId::new(),
        CHILDREN,
        None,
    )
    .await;
}

async fn submit_root<F>(
    journal: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    process_id: ProcessId,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    journal
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit root process");
}

async fn submit_child<F>(
    journal: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
    cap: u32,
    dependency: Option<ProcessDependencySubmission>,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    journal
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(ProcessId::from_uuid(parent_run_id.as_uuid())),
            root_process_id: Some(ProcessId::from_uuid(parent_run_id.as_uuid())),
            spawn_tree_descendant_cap: Some(cap),
            dependency,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit child process");
}

fn turn_scope(thread: &str) -> TurnScope {
    let scope = resource_scope(thread);
    TurnScope::new_with_owner(
        scope.tenant_id,
        scope.agent_id,
        scope.project_id,
        scope.thread_id.expect("thread"),
        Some(scope.user_id),
    )
}

fn resource_scope(thread: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-dependencies").expect("tenant"),
        user_id: UserId::new("user-dependencies").expect("user"),
        agent_id: Some(AgentId::new("agent-dependencies").expect("agent")),
        project_id: Some(ProjectId::new("project-dependencies").expect("project")),
        mission_id: None,
        thread_id: Some(ThreadId::new(thread).expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

fn processes_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("processes alias"),
        VirtualPath::new("/engine/processes").expect("processes target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("processes mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}
