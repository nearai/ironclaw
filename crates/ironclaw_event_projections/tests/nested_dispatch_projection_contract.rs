use std::sync::Arc;

use ironclaw_event_projections::{
    CapabilityActivityProjection, CapabilityActivityStatus, EventProjectionService,
    ProjectionRequest, ProjectionScope, ReplayEventProjectionService, RunProjectionStatus,
    RunStatusProjection,
};
use ironclaw_events::{DurableEventLog, InMemoryDurableEventLog, RuntimeEvent};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, ExtensionId, InvocationId, TenantId, ThreadId, UserId},
    resource::ResourceScope,
    runtime::RuntimeKind,
};

fn scope_for_thread(thread_id: ThreadId, invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: None,
        mission_id: None,
        thread_id: Some(thread_id),
        invocation_id,
    }
}

fn projection_service(log: Arc<InMemoryDurableEventLog>) -> ReplayEventProjectionService {
    ReplayEventProjectionService::new(log)
}

async fn append_nested_dispatch_failure(
    log: &InMemoryDurableEventLog,
    child_scope: ResourceScope,
    parent_invocation_id: InvocationId,
    child_capability: &CapabilityId,
) {
    let provider = ExtensionId::new("script").unwrap();
    for mut event in [
        RuntimeEvent::dispatch_requested(child_scope.clone(), child_capability.clone()),
        RuntimeEvent::runtime_selected(
            child_scope.clone(),
            child_capability.clone(),
            provider.clone(),
            RuntimeKind::Script,
        ),
        RuntimeEvent::dispatch_failed(
            child_scope,
            child_capability.clone(),
            Some(provider),
            Some(RuntimeKind::Script),
            "exit_failure",
        ),
    ] {
        event.parent_invocation_id = Some(parent_invocation_id);
        log.append(event).await.unwrap();
    }
}

fn assert_nested_failure_projection(
    runs: &[RunStatusProjection],
    activities: &[CapabilityActivityProjection],
    parent_invocation_id: InvocationId,
    child_invocation_id: InvocationId,
) {
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].invocation_id, parent_invocation_id);
    assert_eq!(runs[0].status, RunProjectionStatus::Completed);
    assert!(
        runs.iter()
            .all(|run| run.invocation_id != child_invocation_id),
        "nested dispatcher failure must not create a child run row"
    );
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].invocation_id, child_invocation_id);
    assert_eq!(activities[0].status, CapabilityActivityStatus::Failed);
    assert_eq!(activities[0].run_id, Some(parent_invocation_id));
}

#[tokio::test]
async fn runtime_snapshot_keeps_nested_dispatch_failure_out_of_run_status() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let projection = projection_service(Arc::clone(&log));
    let thread_id = ThreadId::new("thread-nested-dispatch-snapshot").unwrap();
    let parent_invocation_id = InvocationId::new();
    let child_invocation_id = InvocationId::new();
    let parent_scope = scope_for_thread(thread_id.clone(), parent_invocation_id);
    let child_scope = scope_for_thread(thread_id, child_invocation_id);
    let child_capability = CapabilityId::new("script.nested").unwrap();

    log.append(RuntimeEvent::model_started(
        parent_scope.clone(),
        CapabilityId::new("loop.model").unwrap(),
    ))
    .await
    .unwrap();
    append_nested_dispatch_failure(
        log.as_ref(),
        child_scope,
        parent_invocation_id,
        &child_capability,
    )
    .await;
    log.append(RuntimeEvent::loop_completed(
        parent_scope.clone(),
        CapabilityId::new("loop.run").unwrap(),
    ))
    .await
    .unwrap();

    let snapshot = projection
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&parent_scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_nested_failure_projection(
        &snapshot.runs,
        &snapshot.capability_activities,
        parent_invocation_id,
        child_invocation_id,
    );
}

#[tokio::test]
async fn runtime_resume_keeps_late_nested_dispatch_failure_out_of_run_status() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let projection = projection_service(Arc::clone(&log));
    let thread_id = ThreadId::new("thread-nested-dispatch-resume").unwrap();
    let parent_invocation_id = InvocationId::new();
    let child_invocation_id = InvocationId::new();
    let parent_scope = scope_for_thread(thread_id.clone(), parent_invocation_id);
    let child_scope = scope_for_thread(thread_id, child_invocation_id);
    let child_capability = CapabilityId::new("script.nested").unwrap();
    let scope = ProjectionScope::from_resource_scope(&parent_scope);

    log.append(RuntimeEvent::model_started(
        parent_scope.clone(),
        CapabilityId::new("loop.model").unwrap(),
    ))
    .await
    .unwrap();
    let initial = projection
        .snapshot(ProjectionRequest {
            scope: scope.clone(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    append_nested_dispatch_failure(
        log.as_ref(),
        child_scope,
        parent_invocation_id,
        &child_capability,
    )
    .await;
    log.append(RuntimeEvent::loop_completed(
        parent_scope,
        CapabilityId::new("loop.run").unwrap(),
    ))
    .await
    .unwrap();

    let replay = projection
        .updates(ProjectionRequest {
            scope,
            after: Some(initial.next_cursor),
            limit: 16,
        })
        .await
        .unwrap();

    assert_nested_failure_projection(
        &replay.runs,
        &replay.capability_activities,
        parent_invocation_id,
        child_invocation_id,
    );
}
