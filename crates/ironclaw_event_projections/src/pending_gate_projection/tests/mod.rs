mod support;

use std::sync::Arc;

use ironclaw_turns::{GateRef, TurnBlockedGateKind, TurnEventKind, TurnRunId, TurnStatus};

use super::*;
use support::*;

#[tokio::test]
async fn blocked_event_upserts_pending_gate_row() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();

    projection
        .project_event_and_advance_cursor(blocked_event(1, scope.clone(), run_id))
        .await
        .unwrap();

    let rows = sink.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.key.tenant_id, scope.tenant_id);
    assert_eq!(row.key.agent_id, scope.agent_id);
    assert_eq!(row.key.project_id, scope.project_id);
    assert_eq!(row.key.owner_user_id.as_str(), "owner-a");
    assert_eq!(row.key.thread_id, scope.thread_id);
    assert_eq!(row.key.run_id, run_id);
    assert_eq!(row.source_cursor, TurnEventCursor(1));
    assert_eq!(row.gate_kind, PendingGateKind::Approval);
    assert_eq!(row.gate_ref.as_str(), "gate:approval-a");
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor(1)
    );
}

#[tokio::test]
async fn publish_projects_blocked_event_without_advancing_replay_cursor() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();

    projection
        .publish(blocked_event(7, scope.clone(), run_id))
        .await
        .unwrap();

    assert_eq!(sink.rows().len(), 1);
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor::default(),
        "live tail projection must not skip durable replay backlog"
    );
}

#[derive(Default)]
struct FailingSink;

#[async_trait]
impl PendingGateProjectionSink for FailingSink {
    async fn upsert_pending_gate(
        &self,
        _row: PendingGateProjectionRow,
    ) -> Result<(), ProjectionError> {
        Err(ProjectionError::Source {
            operation: "failing_upsert",
        })
    }

    async fn remove_pending_gate(
        &self,
        _key: PendingGateProjectionKey,
        _source_cursor: TurnEventCursor,
    ) -> Result<(), ProjectionError> {
        Err(ProjectionError::Source {
            operation: "failing_remove",
        })
    }
}

#[tokio::test]
async fn publish_maps_projection_failure_to_turn_unavailable() {
    let projection = PendingGateProjection::new(
        Arc::new(FailingSink),
        Arc::new(MemoryCursorStore::default()),
    );
    let err = projection
        .publish(blocked_event(1, scope("thread-a"), TurnRunId::new()))
        .await
        .unwrap_err();

    match err {
        ironclaw_turns::TurnError::Unavailable { reason } => {
            assert!(reason.contains("failing_upsert"));
        }
        other => panic!("expected unavailable error, got {other:?}"),
    }
}

#[tokio::test]
async fn publish_maps_invalid_projection_failure_to_turn_invalid_request() {
    let projection = PendingGateProjection::new(
        Arc::new(MemorySink::default()),
        Arc::new(MemoryCursorStore::default()),
    );
    let mut event = blocked_event(1, scope("thread-a"), TurnRunId::new());
    event.blocked_gate = None;

    let err = projection.publish(event).await.unwrap_err();

    match err {
        ironclaw_turns::TurnError::InvalidRequest { reason } => {
            assert!(reason.contains("blocked turn event missing gate metadata"));
        }
        other => panic!("expected invalid request error, got {other:?}"),
    }
}

#[tokio::test]
async fn blocked_event_with_non_projectable_status_does_not_upsert_row() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store);
    let scope = scope("thread-a");

    projection
        .project_event_and_advance_cursor(blocked_event_with(
            1,
            scope,
            TurnRunId::new(),
            TurnStatus::Running,
            TurnBlockedGateKind::Approval,
            "gate:approval-a",
        ))
        .await
        .unwrap();

    assert!(sink.rows().is_empty());
}

#[tokio::test]
async fn malformed_blocked_metadata_errors_without_advancing_cursor() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink, cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();

    for mutate in [
        |event: &mut TurnLifecycleEvent| event.blocked_gate = None,
        |event: &mut TurnLifecycleEvent| event.occurred_at = None,
        |event: &mut TurnLifecycleEvent| event.owner_user_id = None,
    ] {
        let mut event = blocked_event(1, scope.clone(), run_id);
        mutate(&mut event);
        projection
            .project_event_and_advance_cursor(event)
            .await
            .unwrap_err();
        assert_eq!(
            cursor_store
                .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
                .await
                .unwrap(),
            TurnEventCursor::default()
        );
    }
}

#[tokio::test]
async fn non_approval_blocked_events_upsert_expected_gate_kinds() {
    for (status, source_kind, projected_kind, gate_ref) in [
        (
            TurnStatus::BlockedAuth,
            TurnBlockedGateKind::Auth,
            PendingGateKind::Auth,
            "gate:auth-a",
        ),
        (
            TurnStatus::BlockedResource,
            TurnBlockedGateKind::Resource,
            PendingGateKind::Resource,
            "gate:resource-a",
        ),
        (
            TurnStatus::BlockedDependentRun,
            TurnBlockedGateKind::AwaitDependentRun,
            PendingGateKind::AwaitDependentRun,
            "gate:run-a",
        ),
    ] {
        let sink = Arc::new(MemorySink::default());
        let cursor_store = Arc::new(MemoryCursorStore::default());
        let projection = projection(sink.clone(), cursor_store);
        let scope = scope("thread-a");
        let run_id = TurnRunId::new();

        projection
            .project_event_and_advance_cursor(blocked_event_with(
                1,
                scope,
                run_id,
                status,
                source_kind,
                gate_ref,
            ))
            .await
            .unwrap();

        let rows = sink.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].gate_kind, projected_kind);
        assert_eq!(rows[0].gate_ref.as_str(), gate_ref);
    }
}

#[tokio::test]
async fn resumed_and_terminal_events_remove_exact_row() {
    for (status, kind) in [
        (TurnStatus::Queued, TurnEventKind::Resumed),
        (TurnStatus::Completed, TurnEventKind::Completed),
        (TurnStatus::Failed, TurnEventKind::Failed),
        (TurnStatus::Cancelled, TurnEventKind::Cancelled),
    ] {
        let sink = Arc::new(MemorySink::default());
        let cursor_store = Arc::new(MemoryCursorStore::default());
        let projection = projection(sink.clone(), cursor_store);
        let scope = scope("thread-a");
        let run_id = TurnRunId::new();

        projection
            .project_event_and_advance_cursor(blocked_event(1, scope.clone(), run_id))
            .await
            .unwrap();
        projection
            .project_event_and_advance_cursor(lifecycle_event(
                2,
                scope,
                run_id,
                status,
                kind.clone(),
            ))
            .await
            .unwrap();

        assert!(sink.rows().is_empty(), "failed to remove for {kind:?}");
    }
}

#[tokio::test]
async fn replaying_same_stream_is_idempotent() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();
    let source = MemoryTurnEventSource::new(vec![blocked_event(1, scope.clone(), run_id)]);

    projection.replay_scope(&source, &scope, 100).await.unwrap();
    cursor_store.set(
        PENDING_GATE_PROJECTION_CONSUMER_ID,
        &scope,
        TurnEventCursor::default(),
    );
    projection.replay_scope(&source, &scope, 100).await.unwrap();

    assert_eq!(sink.rows().len(), 1);
}

#[tokio::test]
async fn stale_replay_blocked_event_does_not_resurrect_live_removed_gate() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store);
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();

    projection
        .publish(lifecycle_event(
            2,
            scope.clone(),
            run_id,
            TurnStatus::Completed,
            TurnEventKind::Completed,
        ))
        .await
        .unwrap();
    projection
        .project_event(blocked_event(1, scope, run_id))
        .await
        .unwrap();

    assert!(
        sink.rows().is_empty(),
        "older replay upsert must not resurrect a gate removed by newer live delivery"
    );
}

#[tokio::test]
async fn replay_recovers_when_crash_happens_after_row_write_before_cursor_advance() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store);
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();
    let source = MemoryTurnEventSource::new(vec![
        blocked_event(1, scope.clone(), run_id),
        lifecycle_event(
            2,
            scope.clone(),
            run_id,
            TurnStatus::Completed,
            TurnEventKind::Completed,
        ),
    ]);

    sink.upsert_pending_gate(
        row_from_blocked_event(&blocked_event(1, scope.clone(), run_id)).unwrap(),
    )
    .await
    .unwrap();

    let replay = projection.replay_scope(&source, &scope, 100).await.unwrap();

    assert_eq!(replay.processed, 2);
    assert_eq!(replay.next_cursor, TurnEventCursor(2));
    assert!(sink.rows().is_empty());
}

#[tokio::test]
async fn replay_recovers_when_crash_happens_after_remove_before_cursor_advance() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();
    let source = MemoryTurnEventSource::new(vec![
        blocked_event(1, scope.clone(), run_id),
        lifecycle_event(
            2,
            scope.clone(),
            run_id,
            TurnStatus::Completed,
            TurnEventKind::Completed,
        ),
    ]);

    cursor_store.set(
        PENDING_GATE_PROJECTION_CONSUMER_ID,
        &scope,
        TurnEventCursor(1),
    );

    let replay = projection.replay_scope(&source, &scope, 100).await.unwrap();

    assert_eq!(replay.processed, 1);
    assert_eq!(replay.next_cursor, TurnEventCursor(2));
    assert!(sink.rows().is_empty());
}

#[tokio::test]
async fn replay_advances_cursor_once_per_page_after_successful_projection() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink, cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();
    let source = MemoryTurnEventSource::new(vec![
        blocked_event(1, scope.clone(), run_id),
        lifecycle_event(
            2,
            scope.clone(),
            run_id,
            TurnStatus::Completed,
            TurnEventKind::Completed,
        ),
    ]);

    projection.replay_scope(&source, &scope, 100).await.unwrap();

    assert_eq!(cursor_store.advances(), vec![TurnEventCursor(2)]);
}

#[tokio::test]
async fn replay_skips_legacy_events_missing_projection_metadata_and_advances_cursor() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let legacy_run = TurnRunId::new();
    let blocked_run = TurnRunId::new();
    let mut legacy_terminal = lifecycle_event(
        1,
        scope.clone(),
        legacy_run,
        TurnStatus::Completed,
        TurnEventKind::Completed,
    );
    legacy_terminal.owner_user_id = None;
    let source = MemoryTurnEventSource::new(vec![
        legacy_terminal,
        blocked_event(2, scope.clone(), blocked_run),
    ]);

    let replay = projection.replay_scope(&source, &scope, 100).await.unwrap();

    assert_eq!(replay.processed, 2);
    assert_eq!(replay.next_cursor, TurnEventCursor(2));
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor(2)
    );
    let rows = sink.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key.run_id, blocked_run);
}

#[tokio::test]
async fn custom_consumer_id_uses_isolated_replay_cursor() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let scope = scope("thread-a");
    let run_one = TurnRunId::new();
    let run_two = TurnRunId::new();
    let source = MemoryTurnEventSource::new(vec![
        blocked_event(1, scope.clone(), run_one),
        blocked_event(2, scope.clone(), run_two),
    ]);
    cursor_store.set("pending_gate_projection.test.a", &scope, TurnEventCursor(1));
    let projection_a = PendingGateProjection::with_consumer_id(
        "pending_gate_projection.test.a",
        sink.clone(),
        cursor_store.clone(),
    );
    let projection_b = PendingGateProjection::with_consumer_id(
        "pending_gate_projection.test.b",
        sink,
        cursor_store.clone(),
    );

    let replay_a = projection_a
        .replay_scope(&source, &scope, 100)
        .await
        .unwrap();
    let replay_b = projection_b
        .replay_scope(&source, &scope, 100)
        .await
        .unwrap();

    assert_eq!(replay_a.processed, 1);
    assert_eq!(replay_b.processed, 2);
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor("pending_gate_projection.test.a", &scope)
            .await
            .unwrap(),
        TurnEventCursor(2)
    );
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor("pending_gate_projection.test.b", &scope)
            .await
            .unwrap(),
        TurnEventCursor(2)
    );
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor::default()
    );
}

#[test]
fn pending_gate_row_keeps_gate_ref_strongly_typed() {
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();
    let gate_ref = GateRef::new("gate:typed-a").expect("gate ref");
    let event = blocked_event_with(
        1,
        scope,
        run_id,
        TurnStatus::BlockedApproval,
        TurnBlockedGateKind::Approval,
        gate_ref.as_str(),
    );

    let row = row_from_blocked_event(&event).unwrap();

    assert_eq!(row.gate_ref, gate_ref);
}

#[tokio::test]
async fn replay_caps_requested_page_size() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink, cursor_store);
    let scope = scope("thread-a");
    let events = (1..=MAX_TURN_EVENT_PROJECTION_LIMIT as u64 + 1)
        .map(|cursor| blocked_event(cursor, scope.clone(), TurnRunId::new()))
        .collect::<Vec<_>>();
    let source = MemoryTurnEventSource::new(events);

    let replay = projection
        .replay_scope(&source, &scope, MAX_TURN_EVENT_PROJECTION_LIMIT + 10)
        .await
        .unwrap();

    assert_eq!(
        source.requested_limits(),
        vec![MAX_TURN_EVENT_PROJECTION_LIMIT]
    );
    assert_eq!(replay.processed, MAX_TURN_EVENT_PROJECTION_LIMIT);
    assert!(replay.truncated);
}

#[tokio::test]
async fn replay_scope_error_exits_do_not_mutate_rows_or_cursor() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let source = MemoryTurnEventSource::new(Vec::new());

    projection
        .replay_scope(&source, &scope, 0)
        .await
        .unwrap_err();
    projection
        .replay_scope(&FailingTurnEventSource, &scope, 100)
        .await
        .unwrap_err();
    projection
        .replay_scope(&RebaseTurnEventSource, &scope, 100)
        .await
        .unwrap_err();

    assert!(sink.rows().is_empty());
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor::default()
    );
    assert!(cursor_store.advances().is_empty());
}

#[tokio::test]
async fn replay_scope_reports_turn_event_rebase_required() {
    let projection = PendingGateProjection::new(
        Arc::new(MemorySink::default()),
        Arc::new(MemoryCursorStore::default()),
    );
    let scope = scope("thread-a");

    let err = projection
        .replay_scope(&RebaseTurnEventSource, &scope, 100)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ProjectionError::TurnEventRebaseRequired {
            requested: TurnEventCursor(0),
            earliest: TurnEventCursor(5)
        }
    ));
}

#[tokio::test]
async fn cursor_advance_is_monotonic_under_interleaved_replay() {
    let cursor_store = MemoryCursorStore::default();
    let scope = scope("thread-a");

    cursor_store
        .advance_pending_gate_cursor(
            PENDING_GATE_PROJECTION_CONSUMER_ID,
            &scope,
            TurnEventCursor(100),
        )
        .await
        .unwrap();
    cursor_store
        .advance_pending_gate_cursor(
            PENDING_GATE_PROJECTION_CONSUMER_ID,
            &scope,
            TurnEventCursor(1),
        )
        .await
        .unwrap();

    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor(100)
    );
}

#[tokio::test]
async fn terminal_remove_does_not_delete_neighboring_runs() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store);
    let scope = scope("thread-a");
    let removed_run = TurnRunId::new();
    let kept_run = TurnRunId::new();

    projection
        .project_event_and_advance_cursor(blocked_event(1, scope.clone(), removed_run))
        .await
        .unwrap();
    projection
        .project_event_and_advance_cursor(blocked_event(2, scope.clone(), kept_run))
        .await
        .unwrap();
    projection
        .project_event_and_advance_cursor(lifecycle_event(
            3,
            scope,
            removed_run,
            TurnStatus::Completed,
            TurnEventKind::Completed,
        ))
        .await
        .unwrap();

    let rows = sink.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key.run_id, kept_run);
}

#[tokio::test]
async fn terminal_remove_preserves_same_thread_rows_in_other_agent_project_scopes() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store);
    let tenant = TenantId::new("tenant-a").expect("tenant");
    let thread = ThreadId::new("thread-a").expect("thread");
    let scope_a = TurnScope::new(
        tenant.clone(),
        Some(AgentId::new("agent-a").expect("agent")),
        Some(ProjectId::new("project-a").expect("project")),
        thread.clone(),
    );
    let scope_b = TurnScope::new(
        tenant,
        Some(AgentId::new("agent-b").expect("agent")),
        Some(ProjectId::new("project-b").expect("project")),
        thread,
    );
    let removed_run = TurnRunId::new();
    let kept_run = TurnRunId::new();

    projection
        .project_event(blocked_event(1, scope_a.clone(), removed_run))
        .await
        .unwrap();
    projection
        .project_event(blocked_event(2, scope_b, kept_run))
        .await
        .unwrap();
    projection
        .project_event(lifecycle_event(
            3,
            scope_a,
            removed_run,
            TurnStatus::Completed,
            TurnEventKind::Completed,
        ))
        .await
        .unwrap();

    let rows = sink.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key.run_id, kept_run);
    assert_eq!(rows[0].key.agent_id.as_ref().unwrap().as_str(), "agent-b");
    assert_eq!(
        rows[0].key.project_id.as_ref().unwrap().as_str(),
        "project-b"
    );
}

// Cursor store that fails on `advance_pending_gate_cursor` after the first
// successful sink write, used to exercise the post-write cursor-failure path.
struct CursorAdvanceFailingStore;

#[async_trait]
impl PendingGateProjectionCursorStore for CursorAdvanceFailingStore {
    async fn load_pending_gate_cursor(
        &self,
        _consumer_id: &str,
        _scope: &TurnScope,
    ) -> Result<TurnEventCursor, ProjectionError> {
        Ok(TurnEventCursor::default())
    }

    async fn advance_pending_gate_cursor(
        &self,
        _consumer_id: &str,
        _scope: &TurnScope,
        _cursor: TurnEventCursor,
    ) -> Result<(), ProjectionError> {
        Err(ProjectionError::Source {
            operation: "advance_failing",
        })
    }
}

#[tokio::test]
async fn replay_scope_surfaces_cursor_advance_failure_after_sink_writes() {
    let sink = Arc::new(MemorySink::default());
    let projection = PendingGateProjection::new(sink.clone(), Arc::new(CursorAdvanceFailingStore));
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();
    let source = MemoryTurnEventSource::new(vec![blocked_event(1, scope.clone(), run_id)]);

    let err = projection
        .replay_scope(&source, &scope, 100)
        .await
        .expect_err("cursor advance failure must surface");

    assert!(matches!(
        err,
        ProjectionError::Source {
            operation: "advance_failing"
        }
    ));

    // Sink wrote the row; on retry the per-key cursor guard in a real sink
    // suppresses the duplicate upsert. Verifying the row is present
    // exercises the at-least-once contract this finding raises.
    assert_eq!(sink.rows().len(), 1);
}

#[tokio::test]
async fn replay_skips_non_projectable_event_kinds_and_advances_cursor() {
    let sink = Arc::new(MemorySink::default());
    let cursor_store = Arc::new(MemoryCursorStore::default());
    let projection = projection(sink.clone(), cursor_store.clone());
    let scope = scope("thread-a");
    let run_id = TurnRunId::new();

    // Submitted lifecycle event is a non-projectable kind — replay must
    // advance the cursor past it without writing or removing any row.
    let mut submitted = lifecycle_event(
        1,
        scope.clone(),
        run_id,
        TurnStatus::Queued,
        TurnEventKind::Submitted,
    );
    // Drop owner metadata to also verify the silent-skip path does not need
    // projection metadata for non-projectable kinds.
    submitted.owner_user_id = None;
    let source = MemoryTurnEventSource::new(vec![submitted]);

    let replay = projection.replay_scope(&source, &scope, 100).await.unwrap();

    assert_eq!(replay.processed, 1);
    assert_eq!(replay.next_cursor, TurnEventCursor(1));
    assert!(sink.rows().is_empty());
    assert_eq!(
        cursor_store
            .load_pending_gate_cursor(PENDING_GATE_PROJECTION_CONSUMER_ID, &scope)
            .await
            .unwrap(),
        TurnEventCursor(1)
    );
}
