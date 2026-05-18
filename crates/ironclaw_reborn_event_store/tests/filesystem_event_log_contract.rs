//! Contract tests for the filesystem-backed durable event/audit log.
//!
//! These exercise [`FilesystemDurableEventLog`] and
//! [`FilesystemDurableAuditLog`] against an [`InMemoryBackend`] mount. The
//! point is to show that the unified `RootFilesystem::append`/`tail` ops
//! are enough to satisfy the same durable-log contract that the existing
//! SQL backends serve, so the SQL backends can be deleted in task #17
//! without losing coverage.

use std::sync::Arc;

use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventStreamKey, ReadScope,
    RuntimeEvent, RuntimeEventKind,
};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AgentId, AuditEnvelope, AuditEventId, AuditStage,
    CapabilityId, CorrelationId, DecisionSummary, ExtensionId, InvocationId, MountAlias,
    MountGrant, MountPermissions, MountView, ProjectId, ResourceScope, RuntimeKind, TenantId,
    UserId, VirtualPath,
};
use ironclaw_reborn_event_store::{FilesystemDurableAuditLog, FilesystemDurableEventLog};

fn capability_id() -> CapabilityId {
    CapabilityId::new("demo.echo").expect("capability id")
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("demo").expect("extension id")
}

fn scope_for(user: &str, project: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("default").expect("tenant id"),
        user_id: UserId::new(user).expect("user id"),
        agent_id: Some(AgentId::new("default").expect("agent id")),
        project_id: Some(ProjectId::new(project).expect("project id")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn audit_for(scope: &ResourceScope, status: &str) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: CorrelationId::new(),
        stage: AuditStage::After,
        timestamp: chrono::Utc::now(),
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        invocation_id: scope.invocation_id,
        process_id: None,
        approval_request_id: None,
        extension_id: Some(extension_id()),
        action: ActionSummary {
            kind: "dispatch".to_string(),
            target: Some(capability_id().as_str().to_string()),
            effects: Vec::new(),
        },
        decision: DecisionSummary {
            kind: "allow".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(status.to_string()),
            output_bytes: Some(12),
        }),
    }
}

fn build_scoped_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let backend = Arc::new(InMemoryBackend::new());
    // The filesystem log routes every stream through scoped paths under
    // `/events`. Grant the test a `/events` mount with the permissions
    // append+tail need: write (append), read (tail), and list so the
    // resolver doesn't reject metadata operations should we add them.
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/events").expect("alias"),
        VirtualPath::new("/events").expect("target"),
        MountPermissions {
            read: true,
            write: true,
            delete: false,
            list: true,
            execute: false,
        },
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

#[tokio::test]
async fn filesystem_event_log_append_assigns_monotonic_cursors() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope = scope_for("alice", "project-a");

    let entry1 = log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append 1");
    let entry2 = log
        .append(RuntimeEvent::dispatch_succeeded(
            scope.clone(),
            capability_id(),
            extension_id(),
            RuntimeKind::Script,
            7,
        ))
        .await
        .expect("append 2");

    assert!(entry1.cursor < entry2.cursor);
    assert!(entry1.cursor.as_u64() >= 1);
}

#[tokio::test]
async fn filesystem_event_log_replay_returns_records_in_order() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    for _ in 0..3 {
        log.append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append");
    }

    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), None, 10)
        .await
        .expect("replay");
    assert_eq!(replay.entries.len(), 3);
    assert!(replay.entries[0].cursor < replay.entries[1].cursor);
    assert!(replay.entries[1].cursor < replay.entries[2].cursor);
    assert_eq!(replay.next_cursor, replay.entries[2].cursor);
}

#[tokio::test]
async fn filesystem_event_log_replay_filters_by_read_scope() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    log.append(RuntimeEvent::dispatch_requested(
        scope_a.clone(),
        capability_id(),
    ))
    .await
    .expect("append a1");
    log.append(RuntimeEvent::dispatch_requested(
        scope_b.clone(),
        capability_id(),
    ))
    .await
    .expect("append b1");
    log.append(RuntimeEvent::dispatch_succeeded(
        scope_a.clone(),
        capability_id(),
        extension_id(),
        RuntimeKind::Script,
        7,
    ))
    .await
    .expect("append a2");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = log
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("replay");
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(
        replay.entries[1].record.kind,
        RuntimeEventKind::DispatchSucceeded
    );
}

#[tokio::test]
async fn filesystem_event_log_replay_advances_past_trailing_filtered_records() {
    // Regression-shape test mirroring the JSONL/SQL backends: a matched
    // record followed by a filtered one must advance `next_cursor` past
    // the filtered tail so the consumer's resume cursor moves forward.
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    log.append(RuntimeEvent::dispatch_requested(
        scope_a.clone(),
        capability_id(),
    ))
    .await
    .expect("append a");
    log.append(RuntimeEvent::dispatch_requested(
        scope_b.clone(),
        capability_id(),
    ))
    .await
    .expect("append b");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = log
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("replay");
    assert_eq!(replay.entries.len(), 1);
    // Next cursor advanced past the filtered trailing record.
    assert!(replay.next_cursor.as_u64() > replay.entries[0].cursor.as_u64());

    // Follow-up replay from `next_cursor` must not re-surface the filtered
    // record.
    let follow_up = log
        .read_after_cursor(&stream, &project_a, Some(replay.next_cursor), 10)
        .await
        .expect("follow up replay");
    assert!(follow_up.entries.is_empty());
}

#[tokio::test]
async fn filesystem_event_log_rejects_zero_limit() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    let result = log
        .read_after_cursor(&stream, &ReadScope::any(), None, 0)
        .await;
    assert!(matches!(
        result,
        Err(EventError::InvalidReplayRequest { .. })
    ));
}

#[tokio::test]
async fn filesystem_event_log_foreign_future_cursor_is_replay_gap() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    let result = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::new(99)), 10)
        .await;
    match result {
        Err(EventError::ReplayGap {
            requested,
            earliest,
        }) => {
            assert_eq!(requested, EventCursor::new(99));
            assert_eq!(earliest, EventCursor::origin());
        }
        other => panic!("expected replay gap for foreign cursor, got {other:?}"),
    }
}

#[tokio::test]
async fn filesystem_event_log_distinct_streams_are_isolated() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("bob", "project-a");
    let stream_a = EventStreamKey::from_scope(&scope_a);
    let stream_b = EventStreamKey::from_scope(&scope_b);

    log.append(RuntimeEvent::dispatch_requested(
        scope_a.clone(),
        capability_id(),
    ))
    .await
    .expect("append a");
    log.append(RuntimeEvent::dispatch_requested(
        scope_b.clone(),
        capability_id(),
    ))
    .await
    .expect("append b");

    let only_a = log
        .read_after_cursor(&stream_a, &ReadScope::any(), None, 10)
        .await
        .expect("replay a");
    let only_b = log
        .read_after_cursor(&stream_b, &ReadScope::any(), None, 10)
        .await
        .expect("replay b");

    assert_eq!(only_a.entries.len(), 1);
    assert_eq!(only_b.entries.len(), 1);
    // Per-stream cursors are independent (each starts from 1 for its path).
    assert!(only_a.entries[0].cursor.as_u64() >= 1);
    assert!(only_b.entries[0].cursor.as_u64() >= 1);
}

#[tokio::test]
async fn filesystem_audit_log_round_trips_and_filters() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableAuditLog::new(Arc::clone(&fs));
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    log.append(audit_for(&scope_a, "project-a"))
        .await
        .expect("append audit a");
    log.append(audit_for(&scope_b, "project-b"))
        .await
        .expect("append audit b");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = log
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("audit replay");
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(
        replay.entries[0].record.result.as_ref().unwrap().status,
        Some("project-a".to_string())
    );
}

#[tokio::test]
async fn filesystem_event_log_resume_after_cursor_returns_only_new_records() {
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    let e1 = log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append 1");
    let _e2 = log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability_id(),
        ))
        .await
        .expect("append 2");
    let e3 = log
        .append(RuntimeEvent::dispatch_succeeded(
            scope.clone(),
            capability_id(),
            extension_id(),
            RuntimeKind::Script,
            7,
        ))
        .await
        .expect("append 3");

    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(e1.cursor), 10)
        .await
        .expect("resume replay");
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries.last().unwrap().cursor, e3.cursor);
}

#[tokio::test]
async fn filesystem_event_log_caught_up_to_head_returns_empty_not_replay_gap() {
    // Audit finding F1(a): after appending N events, a replay starting from
    // the last entry's cursor must report "caught up to head" — empty entries
    // and `next_cursor == last.cursor` — and must NOT raise ReplayGap. The
    // ReplayGap signal is reserved for cursors that exceed head.
    let fs = build_scoped_fs();
    let log = FilesystemDurableEventLog::new(Arc::clone(&fs));
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    let mut entries = Vec::new();
    for _ in 0..4 {
        let entry = log
            .append(RuntimeEvent::dispatch_requested(
                scope.clone(),
                capability_id(),
            ))
            .await
            .expect("append");
        entries.push(entry);
    }
    let last = entries.last().expect("at least one entry");

    let replay = log
        .read_after_cursor(&stream, &ReadScope::any(), Some(last.cursor), 10)
        .await
        .expect("caught-up replay should not error");
    assert!(
        replay.entries.is_empty(),
        "caught-up-to-head replay must return no entries"
    );
    assert_eq!(
        replay.next_cursor, last.cursor,
        "next_cursor must remain at head when caught up"
    );
}

#[tokio::test]
async fn filesystem_event_log_concurrent_appends_assign_distinct_cursors() {
    // Audit finding F1(b): 8 concurrent appenders against the same stream
    // must each receive a distinct cursor, and the assigned cursors must be
    // strictly increasing (1..=8) once sorted. This guards the per-stream
    // monotonic-cursor invariant under contention.
    let fs = build_scoped_fs();
    let log = Arc::new(FilesystemDurableEventLog::new(Arc::clone(&fs)));
    let scope = scope_for("alice", "project-a");

    let mut tasks = Vec::with_capacity(8);
    for _ in 0..8 {
        let log = Arc::clone(&log);
        let scope = scope.clone();
        tasks.push(tokio::spawn(async move {
            log.append(RuntimeEvent::dispatch_requested(scope, capability_id()))
                .await
                .expect("concurrent append")
        }));
    }

    let mut cursors = Vec::with_capacity(8);
    for task in tasks {
        let entry = task.await.expect("task join");
        cursors.push(entry.cursor.as_u64());
    }
    cursors.sort_unstable();

    // Pairwise-distinct: dedup must not change the length.
    let mut deduped = cursors.clone();
    deduped.dedup();
    assert_eq!(
        deduped.len(),
        cursors.len(),
        "concurrent appends must assign pairwise-distinct cursors, got {cursors:?}"
    );
    // Strictly increasing per stream — for a fresh stream, cursors are 1..=8.
    for window in cursors.windows(2) {
        assert!(
            window[0] < window[1],
            "cursors must be strictly increasing per stream, got {cursors:?}"
        );
    }
}
