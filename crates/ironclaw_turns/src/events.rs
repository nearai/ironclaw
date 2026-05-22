use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, sync::Mutex};
use thiserror::Error;

use ironclaw_host_api::{Timestamp, UserId};

use crate::{GateRef, TurnError, TurnRunId, TurnScope, TurnStatus};

const MAX_IN_MEMORY_EVENTS: usize = 10_000;
pub const MAX_TURN_EVENT_PROJECTION_LIMIT: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct EventCursor(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnEventKind {
    Submitted,
    Resumed,
    RunnerClaimed,
    RunnerHeartbeat,
    RecoveryRequired,
    Blocked,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnBlockedGateKind {
    Approval,
    Auth,
    Resource,
}

impl TurnBlockedGateKind {
    pub fn from_status(status: TurnStatus) -> Option<Self> {
        match status {
            TurnStatus::BlockedApproval => Some(Self::Approval),
            TurnStatus::BlockedAuth => Some(Self::Auth),
            TurnStatus::BlockedResource => Some(Self::Resource),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBlockedGateMetadata {
    pub gate_ref: GateRef,
    pub gate_kind: TurnBlockedGateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnLifecycleEvent {
    pub cursor: EventCursor,
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<UserId>,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub kind: TurnEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_gate: Option<TurnBlockedGateMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sanitized_reason: Option<String>,
}

impl TurnLifecycleEvent {
    /// Return the transport-facing lifecycle event view.
    ///
    /// Internal projection consumers may need gate and owner metadata to
    /// materialize read models, but public lifecycle snapshots must not expose
    /// resolution refs or owner identity.
    pub fn into_public_projection_entry(mut self) -> Self {
        self.blocked_gate = None;
        self.owner_user_id = None;
        self
    }
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError>;
}

#[derive(Default)]
pub struct InMemoryTurnEventSink {
    events: Mutex<Vec<TurnLifecycleEvent>>,
}

impl InMemoryTurnEventSink {
    pub fn events(&self) -> Vec<TurnLifecycleEvent> {
        match self.events.lock() {
            Ok(events) => events.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl TurnEventSink for InMemoryTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        let mut events = self.events.lock().map_err(|_| TurnError::Unavailable {
            reason: "turn event sink mutex poisoned".to_string(),
        })?;
        events.push(event);
        if events.len() > MAX_IN_MEMORY_EVENTS {
            let excess = events.len() - MAX_IN_MEMORY_EVENTS;
            events.drain(0..excess);
        }
        Ok(())
    }
}

/// Scope-bound cursor for transport-agnostic turn lifecycle projections.
///
/// The cursor carries the exact [`TurnScope`] under which it was minted so a
/// product adapter cannot use a cursor from one thread/project as bearer
/// authority to skip or infer events from another scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TurnEventProjectionCursor {
    pub event: EventCursor,
    pub scope: TurnScope,
}

impl TurnEventProjectionCursor {
    pub fn for_scope(scope: TurnScope, event: EventCursor) -> Self {
        Self { event, scope }
    }

    pub fn origin_for_scope(scope: TurnScope) -> Self {
        Self {
            event: EventCursor::default(),
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEventProjectionRequest {
    pub scope: TurnScope,
    pub after: Option<TurnEventProjectionCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEventProjectionSnapshot {
    pub entries: Vec<TurnLifecycleEvent>,
    pub next_cursor: TurnEventProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEventPage {
    pub entries: Vec<TurnLifecycleEvent>,
    pub next_cursor: EventCursor,
    pub truncated: bool,
    pub rebase_required: Option<EventCursor>,
}

#[derive(Debug, Error)]
pub enum TurnEventProjectionError {
    #[error("turn event projection request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("turn event projection rebase required")]
    RebaseRequired {
        requested: Box<TurnEventProjectionCursor>,
        earliest: Box<TurnEventProjectionCursor>,
    },
    #[error("turn event projection source failed during {operation}")]
    Source { operation: &'static str },
}

#[async_trait]
pub trait TurnEventProjectionSource: Send + Sync {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError>;
}

pub struct TurnEventProjectionService<S>
where
    S: TurnEventProjectionSource + ?Sized,
{
    source: Arc<S>,
}

impl<S> TurnEventProjectionService<S>
where
    S: TurnEventProjectionSource + ?Sized,
{
    pub fn new(source: Arc<S>) -> Self {
        Self { source }
    }

    pub async fn snapshot(
        &self,
        request: TurnEventProjectionRequest,
    ) -> Result<TurnEventProjectionSnapshot, TurnEventProjectionError> {
        self.read(request).await
    }

    pub async fn updates(
        &self,
        request: TurnEventProjectionRequest,
    ) -> Result<TurnEventProjectionSnapshot, TurnEventProjectionError> {
        self.read(request).await
    }

    async fn read(
        &self,
        request: TurnEventProjectionRequest,
    ) -> Result<TurnEventProjectionSnapshot, TurnEventProjectionError> {
        if request.limit == 0 || request.limit > MAX_TURN_EVENT_PROJECTION_LIMIT {
            return Err(TurnEventProjectionError::InvalidRequest {
                reason: "limit must be between 1 and MAX_TURN_EVENT_PROJECTION_LIMIT",
            });
        }
        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(TurnEventProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(TurnEventProjectionCursor::origin_for_scope(
                    request.scope.clone(),
                )),
            });
        }
        let after = request.after.as_ref().map(|cursor| cursor.event);
        let page = self
            .source
            .read_turn_events_after(&request.scope, after, request.limit)
            .await
            .map_err(|_| TurnEventProjectionError::Source {
                operation: "read_turn_events_after",
            })?;
        if let Some(rebase_cursor) = page.rebase_required {
            return Err(TurnEventProjectionError::RebaseRequired {
                requested: Box::new(request.after.unwrap_or_else(|| {
                    TurnEventProjectionCursor::origin_for_scope(request.scope.clone())
                })),
                earliest: Box::new(TurnEventProjectionCursor::for_scope(
                    request.scope,
                    rebase_cursor,
                )),
            });
        }
        Ok(TurnEventProjectionSnapshot {
            entries: page
                .entries
                .into_iter()
                .map(TurnLifecycleEvent::into_public_projection_entry)
                .collect(),
            next_cursor: TurnEventProjectionCursor::for_scope(request.scope, page.next_cursor),
            truncated: page.truncated,
        })
    }
}

pub(crate) fn project_turn_events(
    events: &[TurnLifecycleEvent],
    scope: &TurnScope,
    after: Option<EventCursor>,
    limit: usize,
    retention_floor: EventCursor,
) -> TurnEventPage {
    let after = after.unwrap_or_default();
    let mut scoped_events = events
        .iter()
        .filter(|event| &event.scope == scope)
        .cloned()
        .collect::<Vec<_>>();
    scoped_events.sort_by_key(|event| event.cursor);

    let latest_scoped_cursor = scoped_events.last().map(|event| event.cursor);
    let rebase_required = if retention_floor > EventCursor::default() && after < retention_floor {
        Some(retention_floor)
    } else if let Some(latest) = latest_scoped_cursor {
        (after > latest).then_some(latest)
    } else {
        (after > retention_floor).then_some(retention_floor)
    };

    if let Some(rebase_cursor) = rebase_required {
        return TurnEventPage {
            entries: Vec::new(),
            next_cursor: rebase_cursor,
            truncated: false,
            rebase_required: Some(rebase_cursor),
        };
    }

    let mut matching = scoped_events
        .into_iter()
        .filter(|event| event.cursor > after)
        .collect::<Vec<_>>();
    let truncated = matching.len() > limit;
    if truncated {
        matching.truncate(limit);
    }
    let next_cursor = matching.last().map(|event| event.cursor).unwrap_or(after);
    TurnEventPage {
        entries: matching,
        next_cursor,
        truncated,
        rebase_required: None,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

    use crate::{
        GateRef, TurnError, TurnRunId, TurnScope, TurnStatus,
        events::{
            EventCursor, TurnBlockedGateKind, TurnBlockedGateMetadata, TurnEventKind,
            TurnEventPage, TurnEventProjectionService, TurnEventProjectionSource,
            TurnLifecycleEvent, project_turn_events,
        },
    };

    fn scope(thread: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
            ThreadId::new(thread).expect("thread"),
        )
    }

    fn blocked_event(cursor: u64, scope: TurnScope) -> TurnLifecycleEvent {
        TurnLifecycleEvent {
            cursor: EventCursor(cursor),
            scope,
            occurred_at: None,
            owner_user_id: Some(UserId::new("owner-a").expect("owner")),
            run_id: TurnRunId::new(),
            status: TurnStatus::BlockedApproval,
            kind: TurnEventKind::Blocked,
            blocked_gate: Some(TurnBlockedGateMetadata {
                gate_ref: GateRef::new("gate:approval-a").expect("gate ref"),
                gate_kind: TurnBlockedGateKind::Approval,
            }),
            sanitized_reason: Some("approval_required".to_string()),
        }
    }

    struct MemoryProjectionSource {
        events: Vec<TurnLifecycleEvent>,
    }

    #[async_trait]
    impl TurnEventProjectionSource for MemoryProjectionSource {
        async fn read_turn_events_after(
            &self,
            scope: &TurnScope,
            after: Option<EventCursor>,
            limit: usize,
        ) -> Result<TurnEventPage, TurnError> {
            Ok(project_turn_events(
                &self.events,
                scope,
                after,
                limit,
                EventCursor::default(),
            ))
        }
    }

    #[test]
    fn blocked_gate_kind_from_status_ignores_non_blocked_statuses() {
        for status in [
            TurnStatus::Queued,
            TurnStatus::Running,
            TurnStatus::Completed,
            TurnStatus::Failed,
            TurnStatus::Cancelled,
        ] {
            assert_eq!(TurnBlockedGateKind::from_status(status), None);
        }
    }

    #[test]
    fn public_projection_entry_strips_internal_projection_metadata() {
        let event = blocked_event(1, scope("thread-a"));

        let public = event.into_public_projection_entry();

        assert_eq!(public.blocked_gate, None);
        assert_eq!(public.owner_user_id, None);
        assert_eq!(
            public.sanitized_reason.as_deref(),
            Some("approval_required")
        );
    }

    #[test]
    fn project_turn_events_allows_cursor_equal_to_retention_floor() {
        let scope = scope("thread-a");
        let event = blocked_event(6, scope.clone());

        let page = project_turn_events(
            std::slice::from_ref(&event),
            &scope,
            Some(EventCursor(5)),
            10,
            EventCursor(5),
        );

        assert_eq!(page.rebase_required, None);
        assert_eq!(page.entries, vec![event]);
        assert_eq!(page.next_cursor, EventCursor(6));
    }

    #[tokio::test]
    async fn projection_service_strips_internal_projection_metadata_from_snapshot() {
        let scope = scope("thread-a");
        let source = MemoryProjectionSource {
            events: vec![blocked_event(1, scope.clone())],
        };
        let service = TurnEventProjectionService::new(std::sync::Arc::new(source));

        let snapshot = service
            .snapshot(crate::events::TurnEventProjectionRequest {
                scope,
                after: None,
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].blocked_gate, None);
        assert_eq!(snapshot.entries[0].owner_user_id, None);
    }
}
