use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_turns::{GateRef, TurnBlockedGateMetadata, TurnStatus};

use super::super::*;

#[derive(Default)]
pub(super) struct MemorySink {
    rows: Mutex<HashMap<PendingGateProjectionKey, PendingGateProjectionRow>>,
    last_applied: Mutex<HashMap<PendingGateProjectionKey, TurnEventCursor>>,
}

impl MemorySink {
    pub(super) fn rows(&self) -> Vec<PendingGateProjectionRow> {
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
pub(super) struct MemoryCursorStore {
    cursors: Mutex<HashMap<(String, TurnScope), TurnEventCursor>>,
    advances: Mutex<Vec<TurnEventCursor>>,
}

impl MemoryCursorStore {
    pub(super) fn set(&self, consumer_id: &str, scope: &TurnScope, cursor: TurnEventCursor) {
        self.cursors
            .lock()
            .expect("memory cursor lock")
            .insert((consumer_id.to_string(), scope.clone()), cursor);
    }

    pub(super) fn advances(&self) -> Vec<TurnEventCursor> {
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

pub(super) struct MemoryTurnEventSource {
    events: Vec<TurnLifecycleEvent>,
    requested_limits: Mutex<Vec<usize>>,
}

impl MemoryTurnEventSource {
    pub(super) fn new(events: Vec<TurnLifecycleEvent>) -> Self {
        Self {
            events,
            requested_limits: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requested_limits(&self) -> Vec<usize> {
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

pub(super) struct FailingTurnEventSource;

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

pub(super) struct RebaseTurnEventSource;

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

pub(super) fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-a").expect("tenant"),
        Some(AgentId::new("agent-a").expect("agent")),
        Some(ProjectId::new("project-a").expect("project")),
        ThreadId::new(thread).expect("thread"),
    )
}

pub(super) fn blocked_event(
    cursor: u64,
    scope: TurnScope,
    run_id: TurnRunId,
) -> TurnLifecycleEvent {
    blocked_event_with(
        cursor,
        scope,
        run_id,
        TurnStatus::BlockedApproval,
        TurnBlockedGateKind::Approval,
        "gate:approval-a",
    )
}

pub(super) fn blocked_event_with(
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

pub(super) fn lifecycle_event(
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

pub(super) fn projection(
    sink: Arc<MemorySink>,
    cursor_store: Arc<MemoryCursorStore>,
) -> PendingGateProjection {
    PendingGateProjection::new(sink, cursor_store)
}
