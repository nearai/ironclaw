use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_event_projections::{
    AuditProjectionCursor, AuditProjectionError, AuditProjectionRequest, AuditProjectionService,
    AuditProjectionStage, AuditStreamResume, CapabilityActivityStatus, EventProjectionService,
    EventStreamManager, MAX_PROJECTION_PAGE_LIMIT, ProjectionCursor, ProjectionError,
    ProjectionReplay, ProjectionRequest, ProjectionScope, ReplayAuditProjectionService,
    ReplayEventProjectionService, RunProjectionStatus, RuntimeStreamResume, TimelineEntryKind,
};
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay,
    EventStreamKey, InMemoryDurableAuditLog, InMemoryDurableEventLog, ReadScope, RuntimeEvent,
    RuntimeEventId, RuntimeEventKind, UNCLASSIFIED_ERROR_KIND,
};
use ironclaw_host_api::{
    Action, ActionResultSummary, ActionSummary, AgentId, AuditEnvelope, AuditStage, CapabilityId,
    CapabilitySet, CorrelationId, DenyReason, ExtensionId, InvocationId, MountView, ProcessId,
    ProjectId, ResourceScope, RuntimeKind, ScopedPath, TenantId, ThreadId, TrustClass, UserId,
};

#[tokio::test]
async fn replay_audit_projection_preserves_valid_capability_targets() {
    let log = Arc::new(InMemoryDurableAuditLog::new());
    let service = ReplayAuditProjectionService::new(Arc::clone(&log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let action = Action::Dispatch {
        capability: CapabilityId::new("1234567890123456789012345-cap.echo").unwrap(),
        estimated_resources: Default::default(),
    };

    log.append(AuditEnvelope::denied(
        &ctx,
        AuditStage::Denied,
        ActionSummary::from_action(&action),
        DenyReason::PolicyDenied,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&ctx.resource_scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].action_kind, "dispatch");
    assert_eq!(
        snapshot.entries[0].action_target.as_deref(),
        Some("1234567890123456789012345-cap.echo")
    );
}

#[tokio::test]
async fn replay_audit_projection_does_not_expose_unsafe_action_targets() {
    let log = Arc::new(InMemoryDurableAuditLog::new());
    let service = ReplayAuditProjectionService::new(Arc::clone(&log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let action = Action::ReadFile {
        path: ScopedPath::new("/workspace/AUDIT_TARGET_SENTINEL_3022.md").unwrap(),
    };

    log.append(AuditEnvelope::denied(
        &ctx,
        AuditStage::Denied,
        ActionSummary::from_action(&action),
        DenyReason::PolicyDenied,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&ctx.resource_scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].stage, AuditProjectionStage::Denied);
    assert_eq!(snapshot.entries[0].action_kind, "read_file");
    assert_eq!(snapshot.entries[0].action_target, None);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("AUDIT_TARGET_SENTINEL_3022"));
}

#[tokio::test]
async fn replay_audit_projection_preserves_only_safe_obligation_status_labels() {
    let log = Arc::new(InMemoryDurableAuditLog::new());
    let service = ReplayAuditProjectionService::new(Arc::clone(&log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let action = Action::Dispatch {
        capability: capability_id(),
        estimated_resources: Default::default(),
    };
    let mut safe_audit = AuditEnvelope::denied(
        &ctx,
        AuditStage::After,
        ActionSummary::from_action(&action),
        DenyReason::PolicyDenied,
    );
    safe_audit.result = Some(ActionResultSummary {
        success: true,
        status: Some("audit_before,apply_network_policy,inject_secret_once".to_string()),
        output_bytes: Some(10),
    });
    log.append(safe_audit).await.unwrap();

    let mut unsafe_audit = AuditEnvelope::denied(
        &ctx,
        AuditStage::After,
        ActionSummary::from_action(&action),
        DenyReason::PolicyDenied,
    );
    unsafe_audit.result = Some(ActionResultSummary {
        success: false,
        status: Some("api.internal,secret_token".to_string()),
        output_bytes: None,
    });
    log.append(unsafe_audit).await.unwrap();

    let mut duplicate_audit = AuditEnvelope::denied(
        &ctx,
        AuditStage::After,
        ActionSummary::from_action(&action),
        DenyReason::PolicyDenied,
    );
    duplicate_audit.result = Some(ActionResultSummary {
        success: false,
        status: Some("audit_before,audit_before".to_string()),
        output_bytes: None,
    });
    log.append(duplicate_audit).await.unwrap();

    let snapshot = service
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&ctx.resource_scope),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 3);
    assert_eq!(
        snapshot.entries[0].result_status.as_deref(),
        Some("audit_before,apply_network_policy,inject_secret_once")
    );
    assert_eq!(
        snapshot.entries[1].result_status.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
    assert_eq!(
        snapshot.entries[2].result_status.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("api.internal"));
    assert!(!serialized.contains("secret_token"));
}

#[tokio::test]
async fn event_stream_manager_routes_runtime_projection_without_generic_event_payloads() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), audit_log);
    let capability = capability_id();
    let provider = provider_id();
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());

    runtime_log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    runtime_log
        .append(RuntimeEvent::dispatch_succeeded(
            scope.clone(),
            capability.clone(),
            provider,
            RuntimeKind::Script,
            42,
        ))
        .await
        .unwrap();

    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 2);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].capability_id, capability);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(snapshot.next_cursor.runtime, EventCursor::new(2));
}

#[tokio::test]
async fn event_stream_manager_rejects_cross_scope_runtime_resume_cursors() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), audit_log);
    let capability = capability_id();
    let scope_a = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let scope_b = scope_for_thread(ThreadId::new("thread-b").unwrap());

    runtime_log
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    let foreign_event = runtime_log
        .append(RuntimeEvent::dispatch_requested(
            scope_b.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();

    let scope_a_projection = ProjectionScope::from_resource_scope(&scope_a);
    let foreign_cursor = ProjectionCursor::for_scope(
        ProjectionScope::from_resource_scope(&scope_b),
        foreign_event.cursor,
    );
    let error = manager
        .runtime_updates(ProjectionRequest {
            scope: scope_a_projection.clone(),
            after: Some(foreign_cursor.clone()),
            limit: 16,
        })
        .await
        .expect_err("manager must reject cursors minted for a sibling runtime scope");

    match error {
        ProjectionError::RebaseRequired {
            requested,
            earliest,
        } => {
            assert_eq!(*requested, foreign_cursor);
            assert_eq!(earliest.scope, scope_a_projection);
        }
        other => panic!("expected RebaseRequired for cross-scope cursor, got {other:?}"),
    }
}

#[tokio::test]
async fn event_stream_manager_surfaces_domain_rebase_for_stale_resume_cursors() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), Arc::clone(&audit_log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let projection_scope = ProjectionScope::from_resource_scope(&ctx.resource_scope);
    let action = Action::Dispatch {
        capability: capability_id(),
        estimated_resources: Default::default(),
    };

    runtime_log
        .append(RuntimeEvent::dispatch_requested(
            ctx.resource_scope.clone(),
            capability_id(),
        ))
        .await
        .unwrap();
    audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let runtime_error = manager
        .runtime_updates(ProjectionRequest {
            scope: projection_scope.clone(),
            after: Some(ProjectionCursor::for_scope(
                projection_scope.clone(),
                EventCursor::new(99),
            )),
            limit: 16,
        })
        .await
        .expect_err("stale runtime cursor must require rebase");
    assert!(matches!(
        runtime_error,
        ProjectionError::RebaseRequired { .. }
    ));

    let audit_error = manager
        .audit_updates(AuditProjectionRequest {
            scope: projection_scope.clone(),
            after: Some(AuditProjectionCursor::for_scope(
                projection_scope,
                EventCursor::new(99),
            )),
            limit: 16,
        })
        .await
        .expect_err("stale audit cursor must require rebase");
    assert!(matches!(
        audit_error,
        AuditProjectionError::RebaseRequired { .. }
    ));
}

#[tokio::test]
async fn event_stream_manager_resume_without_cursor_returns_initial_snapshots() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), Arc::clone(&audit_log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let projection_scope = ProjectionScope::from_resource_scope(&ctx.resource_scope);
    let action = Action::Dispatch {
        capability: capability_id(),
        estimated_resources: Default::default(),
    };

    runtime_log
        .append(RuntimeEvent::dispatch_requested(
            ctx.resource_scope.clone(),
            capability_id(),
        ))
        .await
        .unwrap();
    audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let runtime_resume = manager
        .runtime_resume(ProjectionRequest {
            scope: projection_scope.clone(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    match runtime_resume {
        RuntimeStreamResume::Snapshot {
            snapshot,
            rebased_from,
            earliest_available,
        } => {
            assert_eq!(snapshot.timeline.entries.len(), 1);
            assert_eq!(rebased_from, None);
            assert_eq!(earliest_available, None);
        }
        other => panic!("expected initial runtime snapshot, got {other:?}"),
    }

    let audit_resume = manager
        .audit_resume(AuditProjectionRequest {
            scope: projection_scope,
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    match audit_resume {
        AuditStreamResume::Snapshot {
            snapshot,
            rebased_from,
            earliest_available,
        } => {
            assert_eq!(snapshot.entries.len(), 1);
            assert_eq!(rebased_from, None);
            assert_eq!(earliest_available, None);
        }
        other => panic!("expected initial audit snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn event_stream_manager_runtime_resume_returns_updates_for_valid_cursor() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), audit_log);
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let projection_scope = ProjectionScope::from_resource_scope(&scope);
    let capability = capability_id();
    let provider = provider_id();

    let first = runtime_log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    runtime_log
        .append(RuntimeEvent::dispatch_succeeded(
            scope.clone(),
            capability,
            provider,
            RuntimeKind::Script,
            12,
        ))
        .await
        .unwrap();

    let resume = manager
        .runtime_resume(ProjectionRequest {
            scope: projection_scope.clone(),
            after: Some(ProjectionCursor::for_scope(projection_scope, first.cursor)),
            limit: 16,
        })
        .await
        .unwrap();

    match resume {
        RuntimeStreamResume::Updates(replay) => {
            assert_eq!(replay.updates.len(), 1);
            assert_eq!(replay.updates[0].kind, TimelineEntryKind::DispatchSucceeded);
            assert_eq!(replay.runs[0].status, RunProjectionStatus::Completed);
        }
        other => panic!("expected updates resume, got {other:?}"),
    }
}

#[tokio::test]
async fn event_stream_manager_runtime_resume_rebases_stale_cursor_to_snapshot() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), audit_log);
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let projection_scope = ProjectionScope::from_resource_scope(&scope);
    let stale_cursor = ProjectionCursor::for_scope(projection_scope.clone(), EventCursor::new(99));

    runtime_log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .unwrap();

    let resume = manager
        .runtime_resume(ProjectionRequest {
            scope: projection_scope.clone(),
            after: Some(stale_cursor.clone()),
            limit: 16,
        })
        .await
        .unwrap();

    match resume {
        RuntimeStreamResume::Snapshot {
            snapshot,
            rebased_from,
            earliest_available,
        } => {
            assert_eq!(rebased_from.as_ref(), Some(&stale_cursor));
            assert_eq!(
                earliest_available.map(|cursor| cursor.scope),
                Some(projection_scope)
            );
            assert_eq!(snapshot.timeline.entries.len(), 1);
            assert_eq!(snapshot.next_cursor.runtime, EventCursor::new(1));
        }
        other => panic!("expected snapshot rebase resume, got {other:?}"),
    }
}

#[tokio::test]
async fn event_stream_manager_runtime_resume_rejects_foreign_cursor_instead_of_snapshotting() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(Arc::clone(&runtime_log), audit_log);
    let scope_a = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let scope_b = scope_for_thread(ThreadId::new("thread-b").unwrap());

    runtime_log
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability_id(),
        ))
        .await
        .unwrap();

    let error = manager
        .runtime_resume(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope_a),
            after: Some(ProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&scope_b),
                EventCursor::new(1),
            )),
            limit: 16,
        })
        .await
        .expect_err("foreign runtime cursor is an authority error, not a rebase snapshot");

    assert!(matches!(error, ProjectionError::RebaseRequired { .. }));
}

#[tokio::test]
async fn event_stream_manager_audit_resume_returns_updates_for_valid_cursor() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(runtime_log, Arc::clone(&audit_log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let projection_scope = ProjectionScope::from_resource_scope(&ctx.resource_scope);
    let action = Action::Dispatch {
        capability: capability_id(),
        estimated_resources: Default::default(),
    };

    let first = audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Before,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();
    audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let resume = manager
        .audit_resume(AuditProjectionRequest {
            scope: projection_scope.clone(),
            after: Some(AuditProjectionCursor::for_scope(
                projection_scope,
                first.cursor,
            )),
            limit: 16,
        })
        .await
        .unwrap();

    match resume {
        AuditStreamResume::Updates(replay) => {
            assert_eq!(replay.entries.len(), 1);
            assert_eq!(replay.entries[0].stage, AuditProjectionStage::Denied);
        }
        other => panic!("expected audit updates resume, got {other:?}"),
    }
}

#[tokio::test]
async fn event_stream_manager_audit_resume_rebases_stale_cursor_to_snapshot() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(runtime_log, Arc::clone(&audit_log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let projection_scope = ProjectionScope::from_resource_scope(&ctx.resource_scope);
    let stale_cursor =
        AuditProjectionCursor::for_scope(projection_scope.clone(), EventCursor::new(99));
    let action = Action::Dispatch {
        capability: capability_id(),
        estimated_resources: Default::default(),
    };

    audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let resume = manager
        .audit_resume(AuditProjectionRequest {
            scope: projection_scope.clone(),
            after: Some(stale_cursor.clone()),
            limit: 16,
        })
        .await
        .unwrap();

    match resume {
        AuditStreamResume::Snapshot {
            snapshot,
            rebased_from,
            earliest_available,
        } => {
            assert_eq!(rebased_from.as_ref(), Some(&stale_cursor));
            assert_eq!(
                earliest_available.map(|cursor| cursor.scope),
                Some(projection_scope)
            );
            assert_eq!(snapshot.entries.len(), 1);
            assert_eq!(snapshot.next_cursor.audit, EventCursor::new(1));
        }
        other => panic!("expected audit snapshot rebase resume, got {other:?}"),
    }
}

#[tokio::test]
async fn event_stream_manager_audit_resume_rejects_foreign_cursor_instead_of_snapshotting() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(runtime_log, audit_log);
    let ctx_a = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let ctx_b = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-b").unwrap()));

    let error = manager
        .audit_resume(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&ctx_a.resource_scope),
            after: Some(AuditProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&ctx_b.resource_scope),
                EventCursor::new(1),
            )),
            limit: 16,
        })
        .await
        .expect_err("foreign audit cursor is an authority error, not a rebase snapshot");

    assert!(matches!(error, AuditProjectionError::RebaseRequired { .. }));
}

#[tokio::test]
async fn event_stream_manager_resume_snapshot_serialization_remains_metadata_only() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(runtime_log, Arc::clone(&audit_log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let action = Action::ReadFile {
        path: ScopedPath::new("/workspace/MANAGER_REBASE_PATH_SENTINEL_3022.md").unwrap(),
    };

    audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let resume = manager
        .audit_resume(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&ctx.resource_scope),
            after: Some(AuditProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&ctx.resource_scope),
                EventCursor::new(99),
            )),
            limit: 16,
        })
        .await
        .unwrap();

    let serialized = serde_json::to_string(&resume).unwrap();
    assert!(!serialized.contains("MANAGER_REBASE_PATH_SENTINEL_3022"));
    assert!(serialized.contains("rebased_from"));
}

#[tokio::test]
async fn event_stream_manager_routes_audit_projection_and_preserves_no_exposure_boundary() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(runtime_log, Arc::clone(&audit_log));
    let ctx = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let action = Action::ReadFile {
        path: ScopedPath::new("/workspace/MANAGER_AUDIT_PATH_SENTINEL_3022.md").unwrap(),
    };

    audit_log
        .append(AuditEnvelope::denied(
            &ctx,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let snapshot = manager
        .audit_snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&ctx.resource_scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].stage, AuditProjectionStage::Denied);
    assert_eq!(snapshot.entries[0].action_kind, "read_file");
    assert_eq!(snapshot.entries[0].action_target, None);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("MANAGER_AUDIT_PATH_SENTINEL_3022"));
}

#[tokio::test]
async fn event_stream_manager_rejects_cross_scope_audit_resume_cursors() {
    let runtime_log = Arc::new(InMemoryDurableEventLog::new());
    let audit_log = Arc::new(InMemoryDurableAuditLog::new());
    let manager = event_stream_manager(runtime_log, Arc::clone(&audit_log));
    let ctx_a = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-a").unwrap()));
    let ctx_b = execution_context_for_scope(scope_for_thread(ThreadId::new("thread-b").unwrap()));
    let action = Action::Dispatch {
        capability: capability_id(),
        estimated_resources: Default::default(),
    };

    audit_log
        .append(AuditEnvelope::denied(
            &ctx_a,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();
    let foreign_entry = audit_log
        .append(AuditEnvelope::denied(
            &ctx_b,
            AuditStage::Denied,
            ActionSummary::from_action(&action),
            DenyReason::PolicyDenied,
        ))
        .await
        .unwrap();

    let scope_a_projection = ProjectionScope::from_resource_scope(&ctx_a.resource_scope);
    let foreign_cursor = AuditProjectionCursor::for_scope(
        ProjectionScope::from_resource_scope(&ctx_b.resource_scope),
        foreign_entry.cursor,
    );
    let error = manager
        .audit_updates(AuditProjectionRequest {
            scope: scope_a_projection.clone(),
            after: Some(foreign_cursor.clone()),
            limit: 16,
        })
        .await
        .expect_err("manager must reject cursors minted for a sibling audit scope");

    match error {
        AuditProjectionError::RebaseRequired {
            requested,
            earliest,
        } => {
            assert_eq!(*requested, foreign_cursor);
            assert_eq!(earliest.scope, scope_a_projection);
        }
        other => panic!("expected audit RebaseRequired for cross-scope cursor, got {other:?}"),
    }
}

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
            after: Some(ProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&scope),
                EventCursor::new(99),
            )),
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
            after: Some(ProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&scope),
                first.cursor,
            )),
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
async fn replay_projection_folds_dispatch_lifecycle_into_capability_activity() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-tool-activity").unwrap());
    let capability = capability_id();
    let provider = provider_id();

    log.append(RuntimeEvent::dispatch_requested(
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
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        42,
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

    assert_eq!(snapshot.capability_activities.len(), 1);
    let activity = &snapshot.capability_activities[0];
    assert_eq!(activity.capability_id, capability);
    assert_eq!(activity.thread_id, scope.thread_id);
    assert_eq!(activity.status, CapabilityActivityStatus::Completed);
    assert_eq!(activity.provider.as_ref(), Some(&provider));
    assert_eq!(activity.runtime, Some(RuntimeKind::Script));
    assert_eq!(activity.output_bytes, Some(42));
    assert_eq!(activity.error_kind, None);
}

#[tokio::test]
async fn replay_projection_folds_process_completed_into_completed_capability_activity() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-tool-activity-process").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_completed(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        process_id,
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

    assert_eq!(snapshot.capability_activities.len(), 1);
    assert_eq!(
        snapshot.capability_activities[0].status,
        CapabilityActivityStatus::Completed
    );
    assert_eq!(
        snapshot.capability_activities[0].process_id,
        Some(process_id)
    );
}

#[tokio::test]
async fn replay_projection_snapshot_bounds_capability_activity_window_to_request_limit() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-tool-activity-window").unwrap());
    let capability = capability_id();
    let mut invocations = Vec::new();

    for _ in 0..5 {
        let invocation_id = InvocationId::new();
        invocations.push(invocation_id);
        let mut invocation_scope = scope.clone();
        invocation_scope.invocation_id = invocation_id;
        log.append(RuntimeEvent::dispatch_requested(
            invocation_scope,
            capability.clone(),
        ))
        .await
        .unwrap();
    }

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 3,
        })
        .await
        .unwrap();

    let activity_invocations = snapshot
        .capability_activities
        .iter()
        .map(|activity| activity.invocation_id)
        .collect::<Vec<_>>();
    assert_eq!(activity_invocations.len(), 3);
    assert!(activity_invocations.contains(&invocations[2]));
    assert!(activity_invocations.contains(&invocations[3]));
    assert!(activity_invocations.contains(&invocations[4]));
    assert!(!activity_invocations.contains(&invocations[0]));
    assert!(!activity_invocations.contains(&invocations[1]));
}

#[tokio::test]
async fn replay_projection_updates_capability_activity_only_for_touched_invocations() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope_a = scope_for_thread(ThreadId::new("thread-tool-activity-a").unwrap());
    let scope_b = scope_for_thread(ThreadId::new("thread-tool-activity-b").unwrap());
    let capability = capability_id();
    let provider = provider_id();

    let first = log
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability.clone(),
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
        capability,
        provider,
        RuntimeKind::Script,
        7,
    ))
    .await
    .unwrap();

    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope_a),
            after: Some(ProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&scope_a),
                first.cursor,
            )),
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(replay.updates.len(), 1);
    assert_eq!(replay.capability_activities.len(), 1);
    assert_eq!(
        replay.capability_activities[0].status,
        CapabilityActivityStatus::Completed
    );
    assert_eq!(replay.capability_activities[0].output_bytes, Some(7));
}

#[tokio::test]
async fn replay_projection_exposes_ordered_capability_activity_transitions() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let thread_id = ThreadId::new("thread-tool-activity-transitions").unwrap();
    let run_scope = scope_for_thread(thread_id.clone());
    let tool_invocation = InvocationId::new();
    let tool_scope = scope_for_thread_with_invocation(thread_id, tool_invocation);
    let capability = capability_id();
    let provider = provider_id();

    let first = log
        .append(RuntimeEvent::model_started(
            run_scope.clone(),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();
    log.append(RuntimeEvent::dispatch_requested(
        tool_scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::runtime_selected(
        tool_scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        tool_scope.clone(),
        capability.clone(),
        provider,
        RuntimeKind::Script,
        7,
    ))
    .await
    .unwrap();

    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&run_scope),
            after: Some(ProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&run_scope),
                first.cursor,
            )),
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(replay.capability_activity_transitions.len(), 2);
    assert_eq!(
        replay
            .capability_activity_transitions
            .iter()
            .map(|transition| transition.invocation_id)
            .collect::<Vec<_>>(),
        vec![tool_invocation, tool_invocation]
    );
    assert_eq!(
        replay
            .capability_activity_transitions
            .iter()
            .map(|transition| transition.status)
            .collect::<Vec<_>>(),
        vec![
            CapabilityActivityStatus::Started,
            CapabilityActivityStatus::Running
        ]
    );
    assert_eq!(
        replay.capability_activity_transitions[0].capability_id,
        capability
    );
    assert_eq!(replay.capability_activities.len(), 1);
    assert_eq!(
        replay.capability_activities[0].status,
        CapabilityActivityStatus::Completed
    );
}

#[test]
fn projection_replay_deserializes_without_capability_activity_transitions() {
    let scope = ProjectionScope::from_resource_scope(&scope_for_thread(
        ThreadId::new("thread-replay-without-transitions").unwrap(),
    ));
    let replay = ProjectionReplay {
        updates: Vec::new(),
        capability_activity_transitions: Vec::new(),
        runs: Vec::new(),
        capability_activities: Vec::new(),
        next_cursor: ProjectionCursor::for_scope(scope, EventCursor::new(1)),
        truncated: false,
    };
    let mut wire = serde_json::to_value(&replay).unwrap();
    wire.as_object_mut()
        .unwrap()
        .remove("capability_activity_transitions");

    let decoded: ProjectionReplay = serde_json::from_value(wire).unwrap();

    assert!(decoded.capability_activity_transitions.is_empty());
}

#[tokio::test]
async fn replay_projection_capability_activity_stays_metadata_only() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-tool-activity-safe").unwrap());

    log.append(RuntimeEvent::dispatch_failed(
        scope.clone(),
        capability_id(),
        Some(provider_id()),
        Some(RuntimeKind::Script),
        "RAW_PROVIDER_ERROR_SENTINEL sk-secret /host/path TOOL_OUTPUT_SENTINEL",
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
    let rendered = serde_json::to_string(&snapshot.capability_activities).unwrap();

    assert!(!rendered.contains("RAW_PROVIDER_ERROR_SENTINEL"));
    assert!(!rendered.contains("TOOL_OUTPUT_SENTINEL"));
    assert!(!rendered.contains("sk-secret"));
    assert!(!rendered.contains("/host/path"));
    assert_eq!(
        snapshot.capability_activities[0].error_kind.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
}

#[tokio::test]
async fn replay_projection_keeps_model_completed_running_until_reply_finalized() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let model_capability = CapabilityId::new("loop.model").unwrap();
    let reply_capability = CapabilityId::new("loop.assistant_reply").unwrap();

    log.append(RuntimeEvent::model_started(
        scope.clone(),
        model_capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::model_completed(
        scope.clone(),
        model_capability.clone(),
    ))
    .await
    .unwrap();

    let after_model_completed = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(after_model_completed.runs.len(), 1);
    assert_eq!(
        after_model_completed.runs[0].status,
        RunProjectionStatus::Running,
        "model_completed only means provider returned; reply finalization can still fail"
    );

    log.append(RuntimeEvent::assistant_reply_finalized(
        scope.clone(),
        reply_capability,
    ))
    .await
    .unwrap();

    let after_reply_finalized = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(after_reply_finalized.runs.len(), 1);
    assert_eq!(
        after_reply_finalized.runs[0].status,
        RunProjectionStatus::Completed
    );
    assert_eq!(
        after_reply_finalized.runs[0].capability_id, model_capability,
        "assistant_reply_finalized must not reclassify the model run capability"
    );
}

#[tokio::test]
async fn replay_projection_keeps_model_failed_non_terminal_until_loop_terminal_event() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-model-retry").unwrap());
    let model_capability = CapabilityId::new("loop.model").unwrap();
    let run_capability = CapabilityId::new("loop.run").unwrap();

    log.append(RuntimeEvent::model_started(
        scope.clone(),
        model_capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::model_failed(
        scope.clone(),
        model_capability.clone(),
        "unavailable",
    ))
    .await
    .unwrap();

    let after_attempt_failure = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(after_attempt_failure.runs.len(), 1);
    assert_eq!(
        after_attempt_failure.runs[0].status,
        RunProjectionStatus::Running,
        "model_failed is attempt-level progress; trusted loop terminal events own run failure"
    );
    assert_eq!(
        after_attempt_failure.runs[0].error_kind.as_deref(),
        Some("unavailable")
    );

    log.append(RuntimeEvent::model_started(
        scope.clone(),
        model_capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::model_completed(
        scope.clone(),
        model_capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::loop_completed(
        scope.clone(),
        run_capability.clone(),
    ))
    .await
    .unwrap();

    let after_terminal_completion = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(after_terminal_completion.runs.len(), 1);
    assert_eq!(
        after_terminal_completion.runs[0].status,
        RunProjectionStatus::Completed
    );
    assert_eq!(
        after_terminal_completion.runs[0].capability_id, model_capability,
        "trusted terminal loop events should not reclassify the primary run capability"
    );
    assert_eq!(
        after_terminal_completion.runs[0].error_kind, None,
        "successful terminal recovery must not expose stale attempt-level model errors"
    );

    let failed_scope = scope_for_thread(ThreadId::new("thread-loop-terminal-failed").unwrap());
    log.append(RuntimeEvent::model_failed(
        failed_scope.clone(),
        CapabilityId::new("loop.model").unwrap(),
        "unavailable",
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::loop_failed(
        failed_scope.clone(),
        run_capability,
        "model_error",
    ))
    .await
    .unwrap();

    let after_terminal_failure = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&failed_scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(after_terminal_failure.runs.len(), 1);
    assert_eq!(
        after_terminal_failure.runs[0].status,
        RunProjectionStatus::Failed
    );
    assert_eq!(
        after_terminal_failure.runs[0].error_kind.as_deref(),
        Some("model_error")
    );
}

#[tokio::test]
async fn replay_projection_projects_loop_cancelled_as_terminal_cancelled_status() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-loop-terminal-cancelled").unwrap());
    let model_capability = CapabilityId::new("loop.model").unwrap();
    let run_capability = CapabilityId::new("loop.run").unwrap();

    log.append(RuntimeEvent::model_started(
        scope.clone(),
        model_capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::loop_cancelled(scope.clone(), run_capability))
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
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Cancelled);
    assert_eq!(
        snapshot.runs[0].capability_id, model_capability,
        "trusted terminal loop events should not reclassify the primary run capability"
    );
    assert_eq!(
        snapshot.runs[0].error_kind, None,
        "cancelled terminal events must clear stale attempt-level errors"
    );
}

#[tokio::test]
async fn replay_projection_updates_preserve_running_process_state_after_checkpoint() {
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
    let started = log
        .append(RuntimeEvent::process_started(
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

    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(ProjectionCursor::for_scope(
                ProjectionScope::from_resource_scope(&scope),
                started.cursor,
            )),
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(replay.updates.len(), 1);
    assert_eq!(replay.updates[0].kind, TimelineEntryKind::DispatchSucceeded);
    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].process_id, Some(process_id));
    assert_eq!(replay.runs[0].status, RunProjectionStatus::Running);
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
    assert_eq!(snapshot.capability_activities.len(), 1);
    assert_eq!(
        snapshot.capability_activities[0].process_id,
        Some(process_id)
    );
    assert_eq!(
        snapshot.capability_activities[0].status,
        CapabilityActivityStatus::Running
    );
}

#[tokio::test]
async fn replay_projection_orders_runs_by_recent_activity_descending() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let thread = ThreadId::new("thread-a").unwrap();
    let older_invocation = InvocationId::parse("00000000-0000-4000-8000-000000000001").unwrap();
    let newer_invocation = InvocationId::parse("ffffffff-ffff-4fff-8fff-ffffffffffff").unwrap();
    let older_scope = scope_for_thread_with_invocation(thread.clone(), older_invocation);
    let newer_scope = scope_for_thread_with_invocation(thread, newer_invocation);
    let capability = capability_id();

    log.append(RuntimeEvent::dispatch_requested(
        older_scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_requested(newer_scope, capability))
        .await
        .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&older_scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 2);
    assert_eq!(snapshot.runs[0].invocation_id, newer_invocation);
    assert_eq!(snapshot.runs[1].invocation_id, older_invocation);
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

    async fn head_cursor(
        &self,
        _stream: &EventStreamKey,
        _after: EventCursor,
    ) -> Result<EventCursor, EventError> {
        Err(EventError::DurableLog {
            reason: "DATABASE_PROJECTION_SENTINEL /tmp/backend-private-path sk_live".to_string(),
        })
    }
}

fn scope_for_thread(thread_id: ThreadId) -> ResourceScope {
    scope_for_thread_with_invocation(thread_id, InvocationId::new())
}

fn scope_for_thread_with_invocation(
    thread_id: ThreadId,
    invocation_id: InvocationId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: Some(thread_id),
        invocation_id,
    }
}

fn execution_context_for_scope(scope: ResourceScope) -> ironclaw_host_api::ExecutionContext {
    let context = ironclaw_host_api::ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Script,
        trust: TrustClass::UserTrusted,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("script").unwrap()
}

fn event_stream_manager(
    runtime_log: Arc<InMemoryDurableEventLog>,
    audit_log: Arc<InMemoryDurableAuditLog>,
) -> EventStreamManager {
    EventStreamManager::new(
        Arc::new(ReplayEventProjectionService::new(runtime_log)),
        Arc::new(ReplayAuditProjectionService::new(audit_log)),
    )
}

// -----------------------------------------------------------------------------
// Regression: PR #3212 review feedback (serrrfirat, 2026-05-03)
// -----------------------------------------------------------------------------

/// Regression for review comment 3178562797: a custom `DurableEventLog`
/// backend can return a `RuntimeEvent` whose `error_kind` was never run
/// through `sanitize_error_kind` (the typed constructors always sanitize, but
/// the struct fields are `pub` so direct construction or a future backend can
/// bypass them). The projection layer must re-sanitize at the projection
/// boundary so leaked paths/secrets cannot reach product DTOs.
#[tokio::test]
async fn replay_projection_re_sanitizes_unsanitized_runtime_events_from_custom_backend() {
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let raw = "raw failure /tmp/private-host-path SECRET_PROJECTION_SENTINEL_sk_live";
    let unsanitized = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::ProcessFailed,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: Some(RuntimeKind::Script),
        process_id: Some(ProcessId::new()),
        output_bytes: None,
        error_kind: Some(raw.to_string()),
        hook_id: None,
        hook_point: None,
        hook_trust_class: None,
        hook_decision: None,
        hook_failure_category: None,
        hook_failure_disposition: None,
    };
    let backend = Arc::new(StaticDurableEventLog {
        entries: vec![EventLogEntry {
            cursor: EventCursor::new(1),
            record: unsanitized,
        }],
    });
    let service = ReplayEventProjectionService::new(Arc::clone(&backend));

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert_eq!(
        snapshot.timeline.entries[0].error_kind.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].error_kind.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    for forbidden in [
        "/tmp/private-host-path",
        "SECRET_PROJECTION_SENTINEL",
        "sk_live",
        "raw failure",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "projection leaked {forbidden}: {serialized}"
        );
    }
}

/// Regression for review comment 3178562826: for a process-backed run, a
/// late `DispatchSucceeded` event must NOT clobber a terminal `Failed` /
/// `Killed` status produced by an earlier `process_failed` /
/// `process_killed` event. The previous guard only preserved `Running`; the
/// `process_started -> process_failed -> dispatch_succeeded` ordering would
/// silently mark the run `Completed` and hide the failure.
#[tokio::test]
async fn replay_projection_dispatch_succeeded_does_not_clobber_terminal_process_failure() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_failed(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
        "process_crashed",
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
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Failed);
    assert_eq!(
        snapshot.runs[0].error_kind.as_deref(),
        Some("process_crashed")
    );
    assert_eq!(snapshot.capability_activities.len(), 1);
    assert_eq!(
        snapshot.capability_activities[0].status,
        CapabilityActivityStatus::Failed
    );
    assert_eq!(
        snapshot.capability_activities[0].error_kind.as_deref(),
        Some("process_crashed")
    );
}

#[tokio::test]
async fn replay_projection_bounded_activity_window_preserves_terminal_process_state() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-bounded-process-state").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_failed(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
        "process_crashed",
    ))
    .await
    .unwrap();

    let mut other_scope = scope.clone();
    other_scope.invocation_id = InvocationId::new();
    log.append(RuntimeEvent::dispatch_requested(
        other_scope,
        capability.clone(),
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
            limit: 1,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.capability_activities.len(), 1);
    assert_eq!(
        snapshot.capability_activities[0].invocation_id,
        scope.invocation_id
    );
    assert_eq!(
        snapshot.capability_activities[0].status,
        CapabilityActivityStatus::Failed,
        "bounded output must not evict lifecycle state before a late dispatch ack is folded"
    );
    assert_eq!(
        snapshot.capability_activities[0].error_kind.as_deref(),
        Some("process_crashed")
    );
}

/// Same regression, but for `process_killed` followed by
/// `dispatch_succeeded`.
#[tokio::test]
async fn replay_projection_dispatch_succeeded_does_not_clobber_terminal_process_killed() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_killed(
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
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Killed);
    assert_eq!(snapshot.capability_activities.len(), 1);
    assert_eq!(
        snapshot.capability_activities[0].status,
        CapabilityActivityStatus::Killed
    );
}

/// Regression for review comment 3178562852: `updates(limit=1)` on a
/// long-lived thread used to read the full prefix into a `Vec` (via the
/// removed `read_runtime_prefix`) before projecting runs. After the fix it
/// folds the prefix incrementally with `O(touched_runs)` allocation, and a
/// hard cap surfaces `RebaseRequired` rather than allocating without
/// bound. This test seeds many prefix events and asserts the bounded-page
/// contract.
#[tokio::test]
async fn replay_projection_updates_with_small_limit_handles_long_prefix() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();

    // Seed many prefix entries (smaller than the rebase cap, larger than the
    // internal page limit so we exercise the paging fold path).
    let prefix_len: usize = 600;
    for _ in 0..prefix_len {
        log.append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    }

    // Resume from "just before the tail" so `updates(limit=1)` returns one
    // new event and must fold the prefix to reconstruct the touched run.
    let resume_after = ProjectionCursor::for_scope(
        ProjectionScope::from_resource_scope(&scope),
        EventCursor::new(prefix_len as u64 - 1),
    );
    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(resume_after),
            limit: 1,
        })
        .await
        .unwrap();

    // The page is bounded to `limit=1`, regardless of prefix length.
    assert_eq!(replay.updates.len(), 1);
    // The single touched run is reconstructed from the folded prefix.
    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].status, RunProjectionStatus::Running);
    assert_eq!(
        replay.next_cursor.runtime,
        EventCursor::new(prefix_len as u64)
    );
}

/// A custom backend that returns a fixed set of (cursor, record) entries on
/// the first `read_after_cursor(after=None, ..)` call and an empty page
/// otherwise. Used for regressions that need to inject hand-built
/// `RuntimeEvent`s that bypass the typed sanitizing constructors.
struct StaticDurableEventLog {
    entries: Vec<EventLogEntry<RuntimeEvent>>,
}

#[async_trait]
impl DurableEventLog for StaticDurableEventLog {
    async fn append(
        &self,
        _event: RuntimeEvent,
    ) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        Err(EventError::DurableLog {
            reason: "static-log:append-not-supported".to_string(),
        })
    }

    async fn read_after_cursor(
        &self,
        _stream: &EventStreamKey,
        _filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        let cutoff = after.unwrap_or_else(EventCursor::origin);
        let visible: Vec<EventLogEntry<RuntimeEvent>> = self
            .entries
            .iter()
            .filter(|entry| entry.cursor > cutoff)
            .take(limit)
            .cloned()
            .collect();
        let next_cursor = visible.last().map(|entry| entry.cursor).unwrap_or(cutoff);
        Ok(EventReplay {
            entries: visible,
            next_cursor,
        })
    }

    async fn head_cursor(
        &self,
        _stream: &EventStreamKey,
        after: EventCursor,
    ) -> Result<EventCursor, EventError> {
        let head = self
            .entries
            .iter()
            .map(|entry| entry.cursor)
            .max()
            .unwrap_or_else(EventCursor::origin);
        if after.as_u64() > head.as_u64() {
            return Err(EventError::ReplayGap {
                requested: after,
                earliest: head,
            });
        }
        Ok(head)
    }
}

// -----------------------------------------------------------------------------
// Regression: PR #3212 review feedback — bounded projection page size
// -----------------------------------------------------------------------------

fn projection_request_with_limit(limit: usize) -> ProjectionRequest {
    ProjectionRequest {
        scope: ProjectionScope::from_resource_scope(&scope_for_thread(
            ThreadId::new("thread-limit").unwrap(),
        )),
        after: None,
        limit,
    }
}

#[tokio::test]
async fn replay_projection_rejects_zero_limit() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .snapshot(projection_request_with_limit(0))
        .await
        .expect_err("limit=0 must be rejected");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

#[tokio::test]
async fn replay_projection_accepts_limit_at_max() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    service
        .snapshot(projection_request_with_limit(MAX_PROJECTION_PAGE_LIMIT))
        .await
        .expect("limit at MAX_PROJECTION_PAGE_LIMIT must be accepted");
}

#[tokio::test]
async fn replay_projection_rejects_limit_above_max() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .snapshot(projection_request_with_limit(MAX_PROJECTION_PAGE_LIMIT + 1))
        .await
        .expect_err("limit > MAX_PROJECTION_PAGE_LIMIT must be rejected");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

#[tokio::test]
async fn replay_projection_rejects_usize_max_limit() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .snapshot(projection_request_with_limit(usize::MAX))
        .await
        .expect_err("limit=usize::MAX must be rejected");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

#[tokio::test]
async fn replay_projection_updates_rejects_limit_above_max() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .updates(projection_request_with_limit(MAX_PROJECTION_PAGE_LIMIT + 1))
        .await
        .expect_err("updates() must enforce the same cap as snapshot()");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

// -----------------------------------------------------------------------------
// Regression: PR #3212 review feedback — projection cursors must be scope-bound
// -----------------------------------------------------------------------------
//
// Cursors returned by the projection service must not be reusable across
// projection scopes. The durable runtime stream is partitioned by
// `(tenant, user, agent)`, while project / mission / thread / process
// filtering happens inside the read filter. A cursor minted while reading
// thread B can carry a runtime number that lies inside the shared stream of
// thread A — passing it to `updates(thread_a_scope, after=cursor_from_b)`
// would make the durable log accept the cursor and return an empty replay,
// silently skipping thread A's earlier events at lower runtime cursors
// instead of forcing a snapshot/rebase.

#[tokio::test]
async fn replay_projection_rejects_cursor_minted_under_a_different_scope() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let capability = capability_id();
    let thread_a = ThreadId::new("thread-a").unwrap();
    let thread_b = ThreadId::new("thread-b").unwrap();
    let scope_a = scope_for_thread(thread_a.clone());
    let scope_b = scope_for_thread(thread_b.clone());

    // Seed thread A first so it has an event at a low runtime cursor that
    // a foreign cursor would otherwise jump over.
    let thread_a_event = log
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    // Seed thread B and capture the cursor it returned.
    let thread_b_event = log
        .append(RuntimeEvent::dispatch_requested(
            scope_b.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    assert!(thread_b_event.cursor > thread_a_event.cursor);

    // Take the cursor minted for thread B's scope and try to use it as the
    // resume cursor for thread A.
    let projection_scope_a = ProjectionScope::from_resource_scope(&scope_a);
    let projection_scope_b = ProjectionScope::from_resource_scope(&scope_b);
    let foreign_cursor = ProjectionCursor::for_scope(projection_scope_b, thread_b_event.cursor);

    let error = service
        .updates(ProjectionRequest {
            scope: projection_scope_a.clone(),
            after: Some(foreign_cursor.clone()),
            limit: 16,
        })
        .await
        .expect_err("cross-scope cursor must be rejected, not silently consumed");

    match error {
        ProjectionError::RebaseRequired {
            requested,
            earliest,
        } => {
            assert_eq!(*requested, foreign_cursor);
            assert_eq!(earliest.scope, projection_scope_a);
        }
        other => panic!("expected RebaseRequired for cross-scope cursor, got {other:?}"),
    }

    // Resume from the legitimate origin for thread A still sees thread A's
    // earlier event — a positive control proving the rejection above did
    // not paper over a real read failure.
    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: projection_scope_a.clone(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert_eq!(
        snapshot.timeline.entries[0].thread_id.as_ref(),
        Some(&thread_a)
    );

    // The same foreign cursor must also be rejected by `snapshot()`.
    let snapshot_error = service
        .snapshot(ProjectionRequest {
            scope: projection_scope_a,
            after: Some(foreign_cursor),
            limit: 16,
        })
        .await
        .expect_err("snapshot() must enforce the same scope binding as updates()");
    assert!(matches!(
        snapshot_error,
        ProjectionError::RebaseRequired { .. }
    ));
}

// -----------------------------------------------------------------------------
// Regression: PR #3212 review feedback — `snapshot()` must not surface stale
// run status when the timeline page is truncated.
// -----------------------------------------------------------------------------
//
// `snapshot()` previously derived `runs` from the single timeline page
// returned by `read_runtime`. A consumer using a snapshot to rebase after
// a replay gap could therefore receive a current-looking
// `RunStatusProjection` whose terminal event was actually sitting on the
// next, unread page — masking failures and leaving UIs/workflows tracking
// long-finished runs as still "Running".

#[tokio::test]
async fn replay_projection_snapshot_runs_reflect_current_stream_head_under_truncation() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();

    // Two events for the same invocation: a Dispatch* request followed by
    // a Dispatch* terminal. The implicit `InvocationId` carried on both
    // matches because they're built from the same `scope`.
    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        7,
    ))
    .await
    .unwrap();

    // `limit=1` truncates the timeline to the first event only — but the
    // run-state projection must still reflect the terminal event sitting
    // on the next page.
    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 1,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert!(snapshot.truncated);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].status,
        RunProjectionStatus::Completed,
        "snapshot must fold runs through stream head; truncated timeline must not leak stale Running status"
    );
}

#[tokio::test]
async fn replay_projection_snapshot_capability_activities_reflect_current_stream_head_under_truncation()
 {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        7,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 1,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert!(snapshot.truncated);
    assert_eq!(snapshot.capability_activities.len(), 1);
    assert_eq!(
        snapshot.capability_activities[0].status,
        CapabilityActivityStatus::Completed,
        "snapshot must fold capability activity through stream head; truncated timeline must not leak stale Started status"
    );
    assert_eq!(snapshot.capability_activities[0].output_bytes, Some(7));
}

#[tokio::test]
async fn replay_projection_snapshot_runs_reflect_process_failed_under_truncation() {
    // Same shape as the previous test but with a `ProcessFailed` terminal
    // event, since the reviewer specifically called out
    // `DispatchSucceeded` / `ProcessFailed` as terminals that get masked
    // when run state is built only from a truncated timeline page.
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_failed(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        process_id,
        "boom",
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 1,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert!(snapshot.truncated);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].status,
        RunProjectionStatus::Failed,
        "snapshot must fold ProcessFailed terminal events through stream head"
    );
    // Sanitization at the projection boundary still applies — the
    // raw `error_kind` is normalized via `sanitize_error_kind`.
    assert!(snapshot.runs[0].error_kind.is_some());
}

// ─── henrypark133 Concerning #6: hook metadata projection ─────────────────

/// Contract test: when the durable event log carries `RuntimeEvent::Hook*`
/// events, the projection's `TimelineEntry` must preserve the sanitized
/// hook metadata (id, point, trust class, decision, failure category/
/// disposition). Without this, product replay sees only "a hook event
/// happened" and cannot identify which hook fired or how it failed
/// (henrypark133 Concerning #6).
#[tokio::test]
async fn hook_runtime_events_project_with_sanitized_hook_metadata() {
    let scope = scope_for_thread(ThreadId::new("thread-hooks").unwrap());

    // Three hook events spanning the full lifecycle: dispatch start,
    // decision emitted, failure recorded.
    let dispatched = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::HookDispatched,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: Some("0123456789abcdef".repeat(4)), // 64-char blake3 hex
        hook_point: Some("before_capability".to_string()),
        hook_trust_class: Some("installed".to_string()),
        hook_decision: None,
        hook_failure_category: None,
        hook_failure_disposition: None,
    };
    let decision = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::HookDecisionEmitted,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: Some("0123456789abcdef".repeat(4)),
        hook_point: None,
        hook_trust_class: None,
        hook_decision: Some("deny".to_string()),
        hook_failure_category: None,
        hook_failure_disposition: None,
    };
    let failed = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::HookFailed,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: Some("fedcba9876543210".repeat(4)),
        hook_point: None,
        hook_trust_class: None,
        hook_decision: None,
        hook_failure_category: Some("timeout".to_string()),
        hook_failure_disposition: Some("fail_closed".to_string()),
    };

    let backend = Arc::new(StaticDurableEventLog {
        entries: vec![
            EventLogEntry {
                cursor: EventCursor::new(1),
                record: dispatched,
            },
            EventLogEntry {
                cursor: EventCursor::new(2),
                record: decision,
            },
            EventLogEntry {
                cursor: EventCursor::new(3),
                record: failed,
            },
        ],
    });
    let service = ReplayEventProjectionService::new(Arc::clone(&backend));

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 3);

    // 1. HookDispatched: hook_id + hook_point + hook_trust_class set.
    let d = &snapshot.timeline.entries[0];
    assert_eq!(d.kind, TimelineEntryKind::HookDispatched);
    assert!(d.hook_id.is_some(), "HookDispatched must carry hook_id");
    assert_eq!(d.hook_point.as_deref(), Some("before_capability"));
    assert_eq!(d.hook_trust_class.as_deref(), Some("installed"));
    assert_eq!(d.hook_decision, None);
    assert_eq!(d.hook_failure_category, None);
    assert_eq!(d.hook_failure_disposition, None);

    // 2. HookDecisionEmitted: hook_id + hook_decision set.
    let e = &snapshot.timeline.entries[1];
    assert_eq!(e.kind, TimelineEntryKind::HookDecisionEmitted);
    assert!(e.hook_id.is_some());
    assert_eq!(e.hook_decision.as_deref(), Some("deny"));

    // 3. HookFailed: hook_id + failure_category + disposition set.
    let f = &snapshot.timeline.entries[2];
    assert_eq!(f.kind, TimelineEntryKind::HookFailed);
    assert!(f.hook_id.is_some());
    assert_eq!(f.hook_failure_category.as_deref(), Some("timeout"));
    assert_eq!(f.hook_failure_disposition.as_deref(), Some("fail_closed"));
}

/// Non-hook events must NOT have hook_* fields populated — guards against
/// a future refactor that accidentally cross-populates the wrong fields.
#[tokio::test]
async fn non_hook_runtime_events_project_with_no_hook_metadata() {
    let scope = scope_for_thread(ThreadId::new("thread-non-hook").unwrap());
    let dispatch_succeeded = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::DispatchSucceeded,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: Some(RuntimeKind::Script),
        process_id: Some(ProcessId::new()),
        output_bytes: Some(42),
        error_kind: None,
        hook_id: None,
        hook_point: None,
        hook_trust_class: None,
        hook_decision: None,
        hook_failure_category: None,
        hook_failure_disposition: None,
    };
    let backend = Arc::new(StaticDurableEventLog {
        entries: vec![EventLogEntry {
            cursor: EventCursor::new(1),
            record: dispatch_succeeded,
        }],
    });
    let service = ReplayEventProjectionService::new(Arc::clone(&backend));

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 1,
        })
        .await
        .unwrap();

    let entry = &snapshot.timeline.entries[0];
    assert_eq!(entry.kind, TimelineEntryKind::DispatchSucceeded);
    assert!(entry.hook_id.is_none());
    assert!(entry.hook_point.is_none());
    assert!(entry.hook_trust_class.is_none());
    assert!(entry.hook_decision.is_none());
    assert!(entry.hook_failure_category.is_none());
    assert!(entry.hook_failure_disposition.is_none());
}

// ─── PR #3573 deferred test: run-status projection of hook events ─────────

/// Deferred from the PR #3573 round-3 review. Pins the contract documented
/// in `run_status_for_event`: hook events are pure observability telemetry
/// and never change a run's lifecycle status. The runner-level test ensures
/// the contract survives via the full snapshot path (durable log →
/// `apply_run_event` → projection), not just the private helper.
///
/// Specifically:
/// - A `HookFailed` event for a run already `Completed` must not downgrade
///   the run to `Failed` (or `Running`).
/// - A `HookDecisionEmitted` event must not change a `Completed` run's
///   status either.
/// - When a run is observed only via hook events (boundary case), the
///   projection defaults to `Running`, never silently dropping the run.
#[tokio::test]
async fn hook_runtime_events_do_not_alter_run_status_projection() {
    let scope = scope_for_thread(ThreadId::new("thread-hook-runs").unwrap());

    // 1. Drive the run to `Completed` via a terminal lifecycle event …
    let loop_completed = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::LoopCompleted,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: None,
        hook_point: None,
        hook_trust_class: None,
        hook_decision: None,
        hook_failure_category: None,
        hook_failure_disposition: None,
    };
    // … then emit hook telemetry that, if the projection mistakenly treated
    // hook events as lifecycle transitions, would either flip the run to
    // `Failed` (HookFailed) or back to `Running` (HookDecisionEmitted).
    let hook_failed_after_completion = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::HookFailed,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: Some("0123456789abcdef".repeat(4)),
        hook_point: None,
        hook_trust_class: None,
        hook_decision: None,
        hook_failure_category: Some("timeout".to_string()),
        hook_failure_disposition: Some("fail_closed".to_string()),
    };
    let hook_decision_after_completion = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::HookDecisionEmitted,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: Some("0123456789abcdef".repeat(4)),
        hook_point: None,
        hook_trust_class: None,
        hook_decision: Some("allow".to_string()),
        hook_failure_category: None,
        hook_failure_disposition: None,
    };

    let backend = Arc::new(StaticDurableEventLog {
        entries: vec![
            EventLogEntry {
                cursor: EventCursor::new(1),
                record: loop_completed,
            },
            EventLogEntry {
                cursor: EventCursor::new(2),
                record: hook_failed_after_completion,
            },
            EventLogEntry {
                cursor: EventCursor::new(3),
                record: hook_decision_after_completion,
            },
        ],
    });
    let service = ReplayEventProjectionService::new(Arc::clone(&backend));
    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 1, "single invocation, single run");
    assert_eq!(
        snapshot.runs[0].status,
        RunProjectionStatus::Completed,
        "post-completion hook events must not alter run status",
    );
}

/// Boundary case: when the only events observed for a run are hook events,
/// the run's status defaults to `Running`. Confirms the projection still
/// surfaces the run rather than silently dropping it (consumers rely on
/// `runs` containing every invocation that produced at least one event).
#[tokio::test]
async fn hook_only_runtime_events_default_run_status_to_running() {
    let scope = scope_for_thread(ThreadId::new("thread-hook-only").unwrap());

    let dispatched = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::HookDispatched,
        scope: scope.clone(),
        parent_invocation_id: None,
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        hook_id: Some("0123456789abcdef".repeat(4)),
        hook_point: Some("before_capability".to_string()),
        hook_trust_class: Some("installed".to_string()),
        hook_decision: None,
        hook_failure_category: None,
        hook_failure_disposition: None,
    };

    let backend = Arc::new(StaticDurableEventLog {
        entries: vec![EventLogEntry {
            cursor: EventCursor::new(1),
            record: dispatched,
        }],
    });
    let service = ReplayEventProjectionService::new(Arc::clone(&backend));
    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 1, "hook-only run still surfaces");
    assert_eq!(
        snapshot.runs[0].status,
        RunProjectionStatus::Running,
        "boundary case: hook-only runs default to Running",
    );
}
