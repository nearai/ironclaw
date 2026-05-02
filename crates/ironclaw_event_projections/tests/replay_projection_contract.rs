use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionRequest, ProjectionScope,
    ReplayEventProjectionService, RunProjectionStatus, TimelineEntryKind,
};
use ironclaw_events::{
    DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay, EventStreamKey,
    InMemoryDurableEventLog, ReadScope, RuntimeEvent,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, ProcessId, ProjectId, ResourceScope,
    RuntimeKind, TenantId, ThreadId, UserId,
};

#[tokio::test]
async fn replay_projection_service_projects_timeline_and_run_status_by_scope() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let capability = capability_id();
    let provider = provider_id();
    let thread_a = ThreadId::new("thread-a").unwrap();
    let thread_b = ThreadId::new("thread-b").unwrap();
    let scope_a = scope_for_thread(thread_a.clone());
    let scope_b = scope_for_thread(thread_b);

    log.append(RuntimeEvent::dispatch_requested(
        scope_a.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::runtime_selected(
        scope_a.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_requested(
        scope_b,
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope_a.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        42,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope_a),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 3);
    assert!(snapshot.timeline.entries.iter().all(|entry| {
        entry.thread_id.as_ref() == Some(&thread_a) && entry.capability_id == capability
    }));
    assert_eq!(
        snapshot
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::DispatchRequested,
            TimelineEntryKind::RuntimeSelected,
            TimelineEntryKind::DispatchSucceeded,
        ]
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(snapshot.next_cursor.runtime, EventCursor::new(4));
    assert!(!snapshot.truncated);
}

#[tokio::test]
async fn replay_projection_updates_return_rebase_signal_for_foreign_or_stale_cursor() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();

    let error = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(ProjectionCursor::new(EventCursor::new(99))),
            limit: 16,
        })
        .await
        .unwrap_err();

    match error {
        ProjectionError::RebaseRequired { requested, .. } => {
            assert_eq!(requested.runtime, EventCursor::new(99));
        }
        other => panic!("expected rebase-required projection error, got {other:?}"),
    }

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.timeline.entries.len(), 1);
}

#[tokio::test]
async fn replay_projection_updates_resume_after_projection_cursor() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();

    let first = log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    log.append(RuntimeEvent::runtime_selected(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        12,
    ))
    .await
    .unwrap();

    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(ProjectionCursor::new(first.cursor)),
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(replay.updates.len(), 2);
    assert_eq!(
        replay
            .updates
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::RuntimeSelected,
            TimelineEntryKind::DispatchSucceeded,
        ]
    );
    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(3));
}

#[tokio::test]
async fn replay_projection_keeps_spawned_process_run_active_until_terminal_process_event() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        0,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].process_id, Some(process_id));
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Running);
}

#[tokio::test]
async fn replay_projection_output_does_not_expose_raw_runtime_details() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();

    log.append(RuntimeEvent::dispatch_failed(
        scope.clone(),
        capability,
        None,
        None,
        "raw failure /tmp/private-host-path SECRET_PROJECTION_SENTINEL_sk_live",
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    let serialized = serde_json::to_string(&snapshot).unwrap();

    for forbidden in [
        "/tmp/private-host-path",
        "SECRET_PROJECTION_SENTINEL",
        "sk_live",
        "raw failure",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "projection output leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("Unclassified"));
}

#[tokio::test]
async fn replay_projection_errors_do_not_expose_backend_details() {
    let service = ReplayEventProjectionService::new(Arc::new(FailingDurableEventLog));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());

    let error = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap_err();
    let message = error.to_string();

    for forbidden in [
        "DATABASE_PROJECTION_SENTINEL",
        "/tmp/backend-private-path",
        "sk_live",
    ] {
        assert!(
            !message.contains(forbidden),
            "projection error leaked {forbidden}: {message}"
        );
    }
    assert!(message.contains("projection source failed"));
}

struct FailingDurableEventLog;

#[async_trait]
impl DurableEventLog for FailingDurableEventLog {
    async fn append(
        &self,
        _event: RuntimeEvent,
    ) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        Err(EventError::DurableLog {
            reason: "DATABASE_PROJECTION_SENTINEL /tmp/backend-private-path sk_live".to_string(),
        })
    }

    async fn read_after_cursor(
        &self,
        _stream: &EventStreamKey,
        _filter: &ReadScope,
        _after: Option<EventCursor>,
        _limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        Err(EventError::DurableLog {
            reason: "DATABASE_PROJECTION_SENTINEL /tmp/backend-private-path sk_live".to_string(),
        })
    }
}

fn scope_for_thread(thread_id: ThreadId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: Some(thread_id),
        invocation_id: InvocationId::new(),
    }
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("script").unwrap()
}
