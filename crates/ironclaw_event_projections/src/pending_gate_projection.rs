use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_turns::{
    EventCursor as TurnEventCursor, MAX_TURN_EVENT_PROJECTION_LIMIT, TurnBlockedGateKind,
    TurnEventKind, TurnEventProjectionSource, TurnEventSink, TurnLifecycleEvent, TurnRunId,
    TurnScope, TurnStatus,
};
use serde::{Deserialize, Serialize};

use crate::ProjectionError;

pub const PENDING_GATE_PROJECTION_CONSUMER_ID: &str = "pending_gate_projection.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PendingGateProjectionGateKind {
    Approval,
    Auth,
    Resource,
}

impl TryFrom<TurnBlockedGateKind> for PendingGateProjectionGateKind {
    type Error = ProjectionError;

    fn try_from(kind: TurnBlockedGateKind) -> Result<Self, Self::Error> {
        Ok(match kind {
            TurnBlockedGateKind::Approval => Self::Approval,
            TurnBlockedGateKind::Auth => Self::Auth,
            TurnBlockedGateKind::Resource => Self::Resource,
            _ => {
                return Err(ProjectionError::InvalidRequest {
                    reason: "unsupported turn blocked gate kind",
                });
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PendingGateProjectionKey {
    pub tenant_id: TenantId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    pub owner_user_id: UserId,
    pub thread_id: ThreadId,
    pub run_id: TurnRunId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGateProjectionRow {
    pub key: PendingGateProjectionKey,
    /// Cursor of the lifecycle event that produced this row.
    ///
    /// Sinks use this as the per-key ordering guard so replay of an older
    /// blocked event cannot resurrect a gate that live delivery already
    /// removed with a newer terminal/resume event.
    pub source_cursor: TurnEventCursor,
    pub gate_kind: PendingGateProjectionGateKind,
    pub gate_ref: String,
    pub blocked_at: Timestamp,
}

#[async_trait]
pub trait PendingGateProjectionSink: Send + Sync {
    /// Upsert a pending-gate row only if `row.source_cursor` is not older than
    /// the last event already applied for `row.key`.
    async fn upsert_pending_gate(
        &self,
        row: PendingGateProjectionRow,
    ) -> Result<(), ProjectionError>;

    /// Remove a pending-gate row only if `source_cursor` is not older than the
    /// last event already applied for `key`.
    async fn remove_pending_gate(
        &self,
        key: PendingGateProjectionKey,
        source_cursor: TurnEventCursor,
    ) -> Result<(), ProjectionError>;
}

#[async_trait]
pub trait PendingGateProjectionCursorStore: Send + Sync {
    /// Load the last durable replay cursor for this consumer and turn scope.
    async fn load_pending_gate_cursor(
        &self,
        consumer_id: &str,
        scope: &TurnScope,
    ) -> Result<TurnEventCursor, ProjectionError>;

    /// Advance the durable replay cursor monotonically.
    ///
    /// Implementations must persist `max(current, cursor)` atomically for the
    /// `(consumer_id, scope)` key. Live [`TurnEventSink`] delivery updates the
    /// read model only and intentionally does not call this method; replay from
    /// the durable turn event source owns cursor progress so it cannot skip a
    /// backlog gap.
    async fn advance_pending_gate_cursor(
        &self,
        consumer_id: &str,
        scope: &TurnScope,
        cursor: TurnEventCursor,
    ) -> Result<(), ProjectionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingGateProjectionReplay {
    pub processed: usize,
    pub next_cursor: TurnEventCursor,
    pub truncated: bool,
}

#[derive(Clone)]
pub struct PendingGateProjection {
    consumer_id: &'static str,
    sink: Arc<dyn PendingGateProjectionSink>,
    cursor_store: Arc<dyn PendingGateProjectionCursorStore>,
}

impl PendingGateProjection {
    pub fn new(
        sink: Arc<dyn PendingGateProjectionSink>,
        cursor_store: Arc<dyn PendingGateProjectionCursorStore>,
    ) -> Self {
        Self {
            consumer_id: PENDING_GATE_PROJECTION_CONSUMER_ID,
            sink,
            cursor_store,
        }
    }

    pub fn with_consumer_id(
        consumer_id: &'static str,
        sink: Arc<dyn PendingGateProjectionSink>,
        cursor_store: Arc<dyn PendingGateProjectionCursorStore>,
    ) -> Self {
        Self {
            consumer_id,
            sink,
            cursor_store,
        }
    }

    pub async fn replay_scope<S>(
        &self,
        source: &S,
        scope: &TurnScope,
        limit: usize,
    ) -> Result<PendingGateProjectionReplay, ProjectionError>
    where
        S: TurnEventProjectionSource + ?Sized,
    {
        if limit == 0 {
            return Err(ProjectionError::InvalidRequest {
                reason: "pending gate replay limit must be greater than zero",
            });
        }

        let after = self
            .cursor_store
            .load_pending_gate_cursor(self.consumer_id, scope)
            .await?;
        let effective_limit = limit.min(MAX_TURN_EVENT_PROJECTION_LIMIT);
        let page = source
            .read_turn_events_after(scope, Some(after), effective_limit)
            .await
            .map_err(|_| ProjectionError::Source {
                operation: "read_turn_events_after",
            })?;

        if page.rebase_required.is_some() {
            return Err(ProjectionError::InvalidRequest {
                reason: "turn event replay requires rebase",
            });
        }

        let mut processed = 0;
        let mut next_cursor = after;
        for event in page.entries {
            next_cursor = event.cursor;
            self.project_event(event, false).await?;
            processed += 1;
        }
        if processed > 0 {
            self.cursor_store
                .advance_pending_gate_cursor(self.consumer_id, scope, next_cursor)
                .await?;
        }

        Ok(PendingGateProjectionReplay {
            processed,
            next_cursor,
            truncated: page.truncated,
        })
    }

    async fn project_event(
        &self,
        event: TurnLifecycleEvent,
        advance_cursor: bool,
    ) -> Result<(), ProjectionError> {
        match event.kind {
            TurnEventKind::Blocked if is_projectable_blocked_status(event.status) => {
                self.sink
                    .upsert_pending_gate(row_from_blocked_event(&event)?)
                    .await?;
            }
            TurnEventKind::Completed
            | TurnEventKind::Failed
            | TurnEventKind::Cancelled
            | TurnEventKind::Resumed => {
                let source_cursor = event.cursor;
                self.sink
                    .remove_pending_gate(key_from_lifecycle_event(&event)?, source_cursor)
                    .await?;
            }
            _ => {}
        }

        if advance_cursor {
            self.cursor_store
                .advance_pending_gate_cursor(self.consumer_id, &event.scope, event.cursor)
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl TurnEventSink for PendingGateProjection {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), ironclaw_turns::TurnError> {
        self.project_event(event, false)
            .await
            .map_err(|_| ironclaw_turns::TurnError::Unavailable {
                reason: "pending gate projection failed".to_string(),
            })
    }
}

fn is_projectable_blocked_status(status: TurnStatus) -> bool {
    matches!(
        status,
        TurnStatus::BlockedApproval | TurnStatus::BlockedAuth | TurnStatus::BlockedResource
    )
}

fn row_from_blocked_event(
    event: &TurnLifecycleEvent,
) -> Result<PendingGateProjectionRow, ProjectionError> {
    let blocked_gate = event
        .blocked_gate
        .as_ref()
        .ok_or(ProjectionError::InvalidRequest {
            reason: "blocked turn event missing gate metadata",
        })?;
    let blocked_at = event.occurred_at.ok_or(ProjectionError::InvalidRequest {
        reason: "blocked turn event missing timestamp",
    })?;

    Ok(PendingGateProjectionRow {
        key: key_from_lifecycle_event(event)?,
        source_cursor: event.cursor,
        gate_kind: blocked_gate.gate_kind.try_into()?,
        gate_ref: blocked_gate.gate_ref.as_str().to_string(),
        blocked_at,
    })
}

fn key_from_lifecycle_event(
    event: &TurnLifecycleEvent,
) -> Result<PendingGateProjectionKey, ProjectionError> {
    let owner_user_id = event
        .owner_user_id
        .clone()
        .ok_or(ProjectionError::InvalidRequest {
            reason: "turn event missing owner metadata",
        })?;

    Ok(PendingGateProjectionKey {
        tenant_id: event.scope.tenant_id.clone(),
        agent_id: event.scope.agent_id.clone(),
        project_id: event.scope.project_id.clone(),
        owner_user_id,
        thread_id: event.scope.thread_id.clone(),
        run_id: event.run_id,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_turns::{GateRef, TurnBlockedGateMetadata};

    use super::*;

    #[derive(Default)]
    struct MemorySink {
        rows: Mutex<HashMap<PendingGateProjectionKey, PendingGateProjectionRow>>,
        last_applied: Mutex<HashMap<PendingGateProjectionKey, TurnEventCursor>>,
    }

    impl MemorySink {
        fn rows(&self) -> Vec<PendingGateProjectionRow> {
            self.rows
                .lock()
                .expect("memory sink lock")
                .values()
                .cloned()
                .collect()
        }

        fn should_apply(&self, key: &PendingGateProjectionKey, cursor: TurnEventCursor) -> bool {
            let mut last_applied = self.last_applied.lock().expect("last applied lock");
            let entry = last_applied.entry(key.clone()).or_default();
            if cursor < *entry {
                return false;
            }
            *entry = cursor;
            true
        }
    }

    #[async_trait]
    impl PendingGateProjectionSink for MemorySink {
        async fn upsert_pending_gate(
            &self,
            row: PendingGateProjectionRow,
        ) -> Result<(), ProjectionError> {
            if !self.should_apply(&row.key, row.source_cursor) {
                return Ok(());
            }
            self.rows
                .lock()
                .expect("memory sink lock")
                .insert(row.key.clone(), row);
            Ok(())
        }

        async fn remove_pending_gate(
            &self,
            key: PendingGateProjectionKey,
            source_cursor: TurnEventCursor,
        ) -> Result<(), ProjectionError> {
            if !self.should_apply(&key, source_cursor) {
                return Ok(());
            }
            self.rows.lock().expect("memory sink lock").remove(&key);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryCursorStore {
        cursors: Mutex<HashMap<(String, TurnScope), TurnEventCursor>>,
        advances: Mutex<Vec<TurnEventCursor>>,
    }

    impl MemoryCursorStore {
        fn set(&self, consumer_id: &str, scope: &TurnScope, cursor: TurnEventCursor) {
            self.cursors
                .lock()
                .expect("memory cursor lock")
                .insert((consumer_id.to_string(), scope.clone()), cursor);
        }

        fn advances(&self) -> Vec<TurnEventCursor> {
            self.advances.lock().expect("advances lock").clone()
        }
    }

    #[async_trait]
    impl PendingGateProjectionCursorStore for MemoryCursorStore {
        async fn load_pending_gate_cursor(
            &self,
            consumer_id: &str,
            scope: &TurnScope,
        ) -> Result<TurnEventCursor, ProjectionError> {
            Ok(*self
                .cursors
                .lock()
                .expect("memory cursor lock")
                .get(&(consumer_id.to_string(), scope.clone()))
                .unwrap_or(&TurnEventCursor::default()))
        }

        async fn advance_pending_gate_cursor(
            &self,
            consumer_id: &str,
            scope: &TurnScope,
            cursor: TurnEventCursor,
        ) -> Result<(), ProjectionError> {
            let mut cursors = self.cursors.lock().expect("memory cursor lock");
            let entry = cursors
                .entry((consumer_id.to_string(), scope.clone()))
                .or_default();
            *entry = (*entry).max(cursor);
            self.advances.lock().expect("advances lock").push(cursor);
            Ok(())
        }
    }

    struct MemoryTurnEventSource {
        events: Vec<TurnLifecycleEvent>,
        requested_limits: Mutex<Vec<usize>>,
    }

    impl MemoryTurnEventSource {
        fn new(events: Vec<TurnLifecycleEvent>) -> Self {
            Self {
                events,
                requested_limits: Mutex::new(Vec::new()),
            }
        }

        fn requested_limits(&self) -> Vec<usize> {
            self.requested_limits
                .lock()
                .expect("requested limits lock")
                .clone()
        }
    }

    #[async_trait]
    impl TurnEventProjectionSource for MemoryTurnEventSource {
        async fn read_turn_events_after(
            &self,
            scope: &TurnScope,
            after: Option<TurnEventCursor>,
            limit: usize,
        ) -> Result<ironclaw_turns::TurnEventPage, ironclaw_turns::TurnError> {
            self.requested_limits
                .lock()
                .expect("requested limits lock")
                .push(limit);
            let after = after.unwrap_or_default();
            let mut entries = self
                .events
                .iter()
                .filter(|event| &event.scope == scope && event.cursor > after)
                .cloned()
                .collect::<Vec<_>>();
            entries.sort_by_key(|event| event.cursor);
            let truncated = entries.len() > limit;
            if truncated {
                entries.truncate(limit);
            }
            let next_cursor = entries.last().map(|event| event.cursor).unwrap_or(after);
            Ok(ironclaw_turns::TurnEventPage {
                entries,
                next_cursor,
                truncated,
                rebase_required: None,
            })
        }
    }

    struct FailingTurnEventSource;

    #[async_trait]
    impl TurnEventProjectionSource for FailingTurnEventSource {
        async fn read_turn_events_after(
            &self,
            _scope: &TurnScope,
            _after: Option<TurnEventCursor>,
            _limit: usize,
        ) -> Result<ironclaw_turns::TurnEventPage, ironclaw_turns::TurnError> {
            Err(ironclaw_turns::TurnError::Unavailable {
                reason: "test source failure".to_string(),
            })
        }
    }

    struct RebaseTurnEventSource;

    #[async_trait]
    impl TurnEventProjectionSource for RebaseTurnEventSource {
        async fn read_turn_events_after(
            &self,
            _scope: &TurnScope,
            _after: Option<TurnEventCursor>,
            _limit: usize,
        ) -> Result<ironclaw_turns::TurnEventPage, ironclaw_turns::TurnError> {
            Ok(ironclaw_turns::TurnEventPage {
                entries: Vec::new(),
                next_cursor: TurnEventCursor(5),
                truncated: false,
                rebase_required: Some(TurnEventCursor(5)),
            })
        }
    }

    fn scope(thread: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
            ThreadId::new(thread).expect("thread"),
        )
    }

    fn blocked_event(cursor: u64, scope: TurnScope, run_id: TurnRunId) -> TurnLifecycleEvent {
        blocked_event_with(
            cursor,
            scope,
            run_id,
            TurnStatus::BlockedApproval,
            TurnBlockedGateKind::Approval,
            "gate:approval-a",
        )
    }

    fn blocked_event_with(
        cursor: u64,
        scope: TurnScope,
        run_id: TurnRunId,
        status: TurnStatus,
        gate_kind: TurnBlockedGateKind,
        gate_ref: &str,
    ) -> TurnLifecycleEvent {
        TurnLifecycleEvent {
            cursor: TurnEventCursor(cursor),
            scope,
            occurred_at: Some(Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3).unwrap()),
            owner_user_id: Some(UserId::new("owner-a").expect("user")),
            run_id,
            status,
            kind: TurnEventKind::Blocked,
            blocked_gate: Some(TurnBlockedGateMetadata {
                gate_ref: GateRef::new(gate_ref).expect("gate ref"),
                gate_kind,
            }),
            sanitized_reason: Some("approval_required".to_string()),
        }
    }

    fn lifecycle_event(
        cursor: u64,
        scope: TurnScope,
        run_id: TurnRunId,
        status: TurnStatus,
        kind: TurnEventKind,
    ) -> TurnLifecycleEvent {
        TurnLifecycleEvent {
            cursor: TurnEventCursor(cursor),
            scope,
            occurred_at: Some(Utc.with_ymd_and_hms(2026, 5, 20, 1, 3, 3).unwrap()),
            owner_user_id: Some(UserId::new("owner-a").expect("user")),
            run_id,
            status,
            kind,
            blocked_gate: None,
            sanitized_reason: None,
        }
    }

    fn projection(
        sink: Arc<MemorySink>,
        cursor_store: Arc<MemoryCursorStore>,
    ) -> PendingGateProjection {
        PendingGateProjection::new(sink, cursor_store)
    }

    #[tokio::test]
    async fn blocked_event_upserts_pending_gate_row() {
        let sink = Arc::new(MemorySink::default());
        let cursor_store = Arc::new(MemoryCursorStore::default());
        let projection = projection(sink.clone(), cursor_store.clone());
        let scope = scope("thread-a");
        let run_id = TurnRunId::new();

        projection
            .project_event(blocked_event(1, scope.clone(), run_id), true)
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
        assert_eq!(row.gate_kind, PendingGateProjectionGateKind::Approval);
        assert_eq!(row.gate_ref, "gate:approval-a");
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

        assert!(matches!(err, ironclaw_turns::TurnError::Unavailable { .. }));
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
            projection.project_event(event, true).await.unwrap_err();
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
    async fn auth_and_resource_blocked_events_upsert_expected_gate_kinds() {
        for (status, source_kind, projected_kind, gate_ref) in [
            (
                TurnStatus::BlockedAuth,
                TurnBlockedGateKind::Auth,
                PendingGateProjectionGateKind::Auth,
                "gate:auth-a",
            ),
            (
                TurnStatus::BlockedResource,
                TurnBlockedGateKind::Resource,
                PendingGateProjectionGateKind::Resource,
                "gate:resource-a",
            ),
        ] {
            let sink = Arc::new(MemorySink::default());
            let cursor_store = Arc::new(MemoryCursorStore::default());
            let projection = projection(sink.clone(), cursor_store);
            let scope = scope("thread-a");
            let run_id = TurnRunId::new();

            projection
                .project_event(
                    blocked_event_with(1, scope, run_id, status, source_kind, gate_ref),
                    true,
                )
                .await
                .unwrap();

            let rows = sink.rows();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].gate_kind, projected_kind);
            assert_eq!(rows[0].gate_ref, gate_ref);
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
                .project_event(blocked_event(1, scope.clone(), run_id), true)
                .await
                .unwrap();
            projection
                .project_event(
                    lifecycle_event(2, scope, run_id, status, kind.clone()),
                    true,
                )
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
            .project_event(blocked_event(1, scope, run_id), false)
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
            .project_event(blocked_event(1, scope.clone(), removed_run), true)
            .await
            .unwrap();
        projection
            .project_event(blocked_event(2, scope.clone(), kept_run), true)
            .await
            .unwrap();
        projection
            .project_event(
                lifecycle_event(
                    3,
                    scope,
                    removed_run,
                    TurnStatus::Completed,
                    TurnEventKind::Completed,
                ),
                true,
            )
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
            .project_event(blocked_event(1, scope_a.clone(), removed_run), false)
            .await
            .unwrap();
        projection
            .project_event(blocked_event(2, scope_b, kept_run), false)
            .await
            .unwrap();
        projection
            .project_event(
                lifecycle_event(
                    3,
                    scope_a,
                    removed_run,
                    TurnStatus::Completed,
                    TurnEventKind::Completed,
                ),
                false,
            )
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
}
