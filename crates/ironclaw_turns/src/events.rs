use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, sync::Mutex};
use thiserror::Error;

use ironclaw_host_api::{RuntimeCredentialAuthRequirement, Timestamp, UserId};

use crate::{
    CapabilityActivityId, GateKind, GateRef, TurnError, TurnRunId, TurnRunState, TurnScope,
    TurnStatus,
};

const MAX_IN_MEMORY_EVENTS: usize = 10_000;
pub const MAX_TURN_EVENT_PROJECTION_LIMIT: usize = 1_000;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
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
    AwaitDependentRun,
    ExternalTool,
}

impl From<GateKind> for TurnBlockedGateKind {
    fn from(kind: GateKind) -> Self {
        match kind {
            GateKind::Approval => Self::Approval,
            GateKind::Auth => Self::Auth,
            GateKind::Resource => Self::Resource,
            GateKind::AwaitDependentRun => Self::AwaitDependentRun,
            GateKind::ExternalTool => Self::ExternalTool,
        }
    }
}

impl TurnBlockedGateKind {
    pub fn from_status(status: TurnStatus) -> Option<Self> {
        GateKind::from_status(status).map(Self::from)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBlockedGateMetadata {
    pub gate_ref: GateRef,
    pub gate_kind: TurnBlockedGateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<CapabilityActivityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
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
    // `skip_serializing_if` matches the other optional fields above:
    // pre-projection event rows emitted `"sanitized_reason": null`; new rows
    // omit the field entirely. `serde(default)` rehydrates omitted fields as
    // `None`, so the wire change is round-trip safe in both directions and
    // adapters that persist `TurnLifecycleEvent` do not need a migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sanitized_reason: Option<String>,
    /// Present only on `Failed` events: whether the failed run recorded a
    /// resumable checkpoint and can be retried. Same serde shape as
    /// `sanitized_reason`, so persisted pre-retry event rows rehydrate as
    /// `None` without migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    /// Secret-scrubbed, model-visible raw cause for a failed run, sourced from
    /// the failure record's detail channel. Distinct from `sanitized_reason`
    /// (which is the bounded category): this carries the real fault text so the
    /// failure explainer can describe it. Same serde shape as the other
    /// optional fields, so persisted pre-detail event rows rehydrate as `None`
    /// without migration. This is the internal event record; public projections
    /// do not expose it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl TurnLifecycleEvent {
    pub fn from_run_state(
        state: &TurnRunState,
        kind: TurnEventKind,
        sanitized_reason: Option<String>,
    ) -> Self {
        let blocked_gate = if kind == TurnEventKind::Blocked {
            state.gate_ref.clone().and_then(|gate_ref| {
                TurnBlockedGateKind::from_status(state.status).map(|gate_kind| {
                    TurnBlockedGateMetadata {
                        gate_ref,
                        gate_kind,
                        activity_id: state.blocked_activity_id,
                        credential_requirements: state.credential_requirements.clone(),
                    }
                })
            })
        } else {
            None
        };
        let retryable = (kind == TurnEventKind::Failed).then(|| state.checkpoint_id.is_some());
        let detail = (kind == TurnEventKind::Failed)
            .then(|| {
                state
                    .failure
                    .as_ref()
                    .and_then(|failure| failure.detail().map(str::to_string))
            })
            .flatten();
        Self {
            cursor: state.event_cursor,
            scope: state.scope.clone(),
            occurred_at: Some(Utc::now()),
            owner_user_id: lifecycle_owner_user_id(
                &state.scope,
                state.actor.as_ref().map(|actor| &actor.user_id),
            ),
            run_id: state.run_id,
            status: state.status,
            kind,
            blocked_gate,
            sanitized_reason,
            retryable,
            detail,
        }
    }

    /// Return the transport-facing lifecycle event view.
    ///
    /// `TurnLifecycleEvent` is the internal event record and may carry raw
    /// resolver refs. Public lifecycle projections expose only redacted status
    /// facts; reducers that need raw refs must use [`TurnEventReducerService`].
    pub fn into_public_projection_entry(self) -> TurnLifecycleProjectionEntry {
        TurnLifecycleProjectionEntry {
            cursor: self.cursor,
            scope: self.scope,
            occurred_at: self.occurred_at,
            run_id: self.run_id,
            status: self.status,
            kind: self.kind,
            sanitized_reason: self.sanitized_reason,
        }
    }
}

pub(crate) fn lifecycle_owner_user_id(
    scope: &TurnScope,
    fallback_actor_user_id: Option<&UserId>,
) -> Option<UserId> {
    scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| fallback_actor_user_id.cloned())
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError>;
}

#[async_trait]
pub trait TurnCommittedEventObserver: Send + Sync {
    /// Returns true when this observer must process a committed state.
    ///
    /// Observer errors are treated as required side-effect failures by most
    /// coordinator and runner transitions. Callers that have already committed
    /// a lease batch may log observer failures and return the committed state
    /// so the runner does not lose track of persisted ownership.
    fn observes_state(&self, _state: &TurnRunState) -> bool {
        true
    }

    /// Returns true when this observer must process a committed event.
    ///
    /// Event observers run before best-effort event sinks. Errors are
    /// propagated to the coordinator caller because the observer represents an
    /// internal consistency side effect rather than external notification.
    fn observes_event(&self, _event: &TurnLifecycleEvent) -> bool {
        true
    }

    async fn observe_committed_state(&self, state: TurnRunState) -> Result<(), TurnError>;

    async fn observe_committed_event(&self, event: TurnLifecycleEvent) -> Result<(), TurnError>;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<UserId>,
    pub after: Option<TurnEventProjectionCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnLifecycleProjectionEntry {
    pub cursor: EventCursor,
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<Timestamp>,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub kind: TurnEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sanitized_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEventProjectionSnapshot {
    pub entries: Vec<TurnLifecycleProjectionEntry>,
    pub next_cursor: TurnEventProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEventReducerSnapshot {
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
    #[error("turn event projection source failed during {operation}: {reason}")]
    Source {
        operation: &'static str,
        #[source]
        reason: TurnError,
    },
}

#[async_trait]
pub trait TurnEventProjectionSource: Send + Sync {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError>;

    /// Read the authoritative lifecycle log in global cursor order for a
    /// host-owned durable projection consumer.
    ///
    /// Implementations must use an indexed, bounded read and fail explicitly
    /// rather than fall back to an unbounded directory scan. The consumer owns
    /// its durable cursor and advances it only after derived state commits. A
    /// retention gap is surfaced through [`TurnEventPage::rebase_required`]
    /// and must never be silently skipped.
    #[doc(hidden)]
    async fn read_turn_event_log_after(
        &self,
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

pub struct TurnEventReducerService<S>
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
        read_public(self.source.as_ref(), request).await
    }

    pub async fn updates(
        &self,
        request: TurnEventProjectionRequest,
    ) -> Result<TurnEventProjectionSnapshot, TurnEventProjectionError> {
        read_public(self.source.as_ref(), request).await
    }
}

impl<S> TurnEventReducerService<S>
where
    S: TurnEventProjectionSource + ?Sized,
{
    pub fn new(source: Arc<S>) -> Self {
        Self { source }
    }

    /// Return raw scoped lifecycle events for projection reducers.
    ///
    /// This service preserves internal resolver refs such as gate refs so reducers
    /// can materialize product-specific read models. Do not expose its entries
    /// directly across transport or product API boundaries.
    pub async fn snapshot(
        &self,
        request: TurnEventProjectionRequest,
    ) -> Result<TurnEventReducerSnapshot, TurnEventProjectionError> {
        read_reducer(self.source.as_ref(), request).await
    }

    /// Return raw scoped lifecycle updates for projection reducers.
    ///
    /// See [`Self::snapshot`] for the boundary contract.
    pub async fn updates(
        &self,
        request: TurnEventProjectionRequest,
    ) -> Result<TurnEventReducerSnapshot, TurnEventProjectionError> {
        read_reducer(self.source.as_ref(), request).await
    }
}

async fn read_public<S>(
    source: &S,
    request: TurnEventProjectionRequest,
) -> Result<TurnEventProjectionSnapshot, TurnEventProjectionError>
where
    S: TurnEventProjectionSource + ?Sized,
{
    let snapshot = read_reducer(source, request).await?;
    Ok(TurnEventProjectionSnapshot {
        entries: snapshot
            .entries
            .into_iter()
            .map(TurnLifecycleEvent::into_public_projection_entry)
            .collect(),
        next_cursor: snapshot.next_cursor,
        truncated: snapshot.truncated,
    })
}

async fn read_reducer<S>(
    source: &S,
    request: TurnEventProjectionRequest,
) -> Result<TurnEventReducerSnapshot, TurnEventProjectionError>
where
    S: TurnEventProjectionSource + ?Sized,
{
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
    let page = source
        .read_turn_events_after(
            &request.scope,
            request.owner_user_id.as_ref(),
            after,
            request.limit,
        )
        .await
        .map_err(|reason| TurnEventProjectionError::Source {
            operation: "read_turn_events_after",
            reason,
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
    Ok(TurnEventReducerSnapshot {
        entries: page.entries,
        next_cursor: TurnEventProjectionCursor::for_scope(request.scope, page.next_cursor),
        truncated: page.truncated,
    })
}

pub(crate) fn project_turn_events(
    events: &[TurnLifecycleEvent],
    scope: &TurnScope,
    owner_user_id: Option<&UserId>,
    after: Option<EventCursor>,
    limit: usize,
    retention_floor: EventCursor,
) -> TurnEventPage {
    let after = after.unwrap_or_default();
    let mut scoped_events = events
        .iter()
        .filter(|event| {
            &event.scope == scope
                && owner_user_id.is_none_or(|owner| event.owner_user_id.as_ref() == Some(owner))
        })
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
    use ironclaw_host_api::{
        AgentId, ExtensionId, ProjectId, RuntimeCredentialAuthRequirement, TenantId, ThreadId,
        UserId, VendorId,
    };

    use crate::{
        AcceptedMessageRef, CapabilityActivityId, GateRef, ReplyTargetBindingRef, RunProfileId,
        RunProfileVersion, SourceBindingRef, TurnActor, TurnError, TurnId, TurnRunId, TurnRunState,
        TurnScope, TurnStatus,
        events::{
            EventCursor, TurnBlockedGateKind, TurnBlockedGateMetadata, TurnEventKind,
            TurnEventPage, TurnEventProjectionError, TurnEventProjectionService,
            TurnEventProjectionSource, TurnEventReducerService, TurnLifecycleEvent,
            project_turn_events,
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
                activity_id: None,
                credential_requirements: vec![RuntimeCredentialAuthRequirement {
                    provider: VendorId::new("github").expect("provider"),
                    setup: Default::default(),
                    requester_extension: ExtensionId::new("github").expect("extension"),
                    provider_scopes: vec!["repo".to_string()],
                }],
            }),
            sanitized_reason: Some("approval_required".to_string()),
            retryable: None,
            detail: None,
        }
    }

    fn blocked_event_with_activity(
        cursor: u64,
        scope: TurnScope,
        activity_id: CapabilityActivityId,
    ) -> TurnLifecycleEvent {
        let mut event = blocked_event(cursor, scope);
        event
            .blocked_gate
            .as_mut()
            .expect("blocked event has gate metadata")
            .activity_id = Some(activity_id);
        event
    }

    struct MemoryProjectionSource {
        events: Vec<TurnLifecycleEvent>,
    }

    #[async_trait]
    impl TurnEventProjectionSource for MemoryProjectionSource {
        async fn read_turn_events_after(
            &self,
            scope: &TurnScope,
            owner_user_id: Option<&UserId>,
            after: Option<EventCursor>,
            limit: usize,
        ) -> Result<TurnEventPage, TurnError> {
            Ok(project_turn_events(
                &self.events,
                scope,
                owner_user_id,
                after,
                limit,
                EventCursor::default(),
            ))
        }

        async fn read_turn_event_log_after(
            &self,
            after: Option<EventCursor>,
            limit: usize,
        ) -> Result<TurnEventPage, TurnError> {
            let after = after.unwrap_or_default();
            let mut entries = self
                .events
                .iter()
                .filter(|event| event.cursor > after)
                .cloned()
                .collect::<Vec<_>>();
            entries.sort_by_key(|event| event.cursor);
            let truncated = entries.len() > limit;
            if truncated {
                entries.truncate(limit);
            }
            let next_cursor = entries.last().map_or(after, |event| event.cursor);
            Ok(TurnEventPage {
                entries,
                next_cursor,
                truncated,
                rebase_required: None,
            })
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
    fn external_tool_gate_kind_maps_from_status_and_round_trips() {
        assert_eq!(
            TurnBlockedGateKind::from_status(TurnStatus::BlockedExternalTool),
            Some(TurnBlockedGateKind::ExternalTool)
        );
        // Wire-stable snake_case contract for the new gate kind.
        let wire = serde_json::to_string(&TurnBlockedGateKind::ExternalTool).expect("serialize");
        assert_eq!(wire, "\"external_tool\"");
        let parsed: TurnBlockedGateKind = serde_json::from_str(&wire).expect("deserialize");
        assert_eq!(parsed, TurnBlockedGateKind::ExternalTool);
    }

    #[test]
    fn run_state_lifecycle_event_prefers_explicit_owner_over_actor() {
        let actor = UserId::new("user:actor").expect("actor");
        let owner = UserId::new("user:subject").expect("subject");
        let scope = TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("tenant"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
            ThreadId::new("thread-a").expect("thread"),
            Some(owner.clone()),
        );
        let activity_id = CapabilityActivityId::new();
        let state = TurnRunState {
            scope,
            actor: Some(TurnActor::new(actor)),
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::BlockedAuth,
            accepted_message_ref: AcceptedMessageRef::new("accepted-a").expect("accepted ref"),
            source_binding_ref: SourceBindingRef::new("source-a").expect("source ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-a").expect("reply ref"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            model_usage: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: Some(GateRef::new("gate:auth-a").expect("gate ref")),
            blocked_activity_id: Some(activity_id),
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(1),
            product_context: None,
            resume_disposition: None,
        };

        let event = TurnLifecycleEvent::from_run_state(&state, TurnEventKind::Blocked, None);

        assert_eq!(event.owner_user_id, Some(owner));
        assert_eq!(
            event
                .blocked_gate
                .as_ref()
                .expect("blocked run projects gate metadata")
                .activity_id,
            Some(activity_id)
        );
    }

    #[test]
    fn failed_run_state_lifecycle_event_carries_failure_detail() {
        let scope = scope("thread-detail");
        let state = TurnRunState {
            scope,
            actor: Some(TurnActor::new(UserId::new("user:actor").expect("actor"))),
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Failed,
            accepted_message_ref: AcceptedMessageRef::new("accepted-a").expect("accepted ref"),
            source_binding_ref: SourceBindingRef::new("source-a").expect("source ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-a").expect("reply ref"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            model_usage: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: Some(
                crate::SanitizedFailure::new("model_unavailable")
                    .expect("category")
                    .with_detail("HTTP 404 model not found"),
            ),
            event_cursor: EventCursor(1),
            product_context: None,
            resume_disposition: None,
        };

        let event = TurnLifecycleEvent::from_run_state(
            &state,
            TurnEventKind::Failed,
            Some("model_unavailable".to_string()),
        );

        assert_eq!(event.detail.as_deref(), Some("HTTP 404 model not found"));
    }

    #[test]
    fn lifecycle_event_legacy_row_without_detail_round_trips() {
        // Persisted pre-detail event rows omit the field; `serde(default)` must
        // rehydrate `detail == None` and `skip_serializing_if` must not emit it.
        let json = r#"{"cursor":7,"scope":{"tenant_id":"tenant-a","agent_id":"agent-a","project_id":"project-a","thread_id":"thread-a"},"run_id":"00000000-0000-0000-0000-000000000001","status":"Failed","kind":"Failed"}"#;
        let event: TurnLifecycleEvent =
            serde_json::from_str(json).expect("legacy event row must deserialize");
        assert_eq!(event.detail, None);

        let reserialized = serde_json::to_string(&event).expect("serialize");
        assert!(
            !reserialized.contains("detail"),
            "absent detail must not be re-introduced on serialize: {reserialized}"
        );
    }

    #[test]
    fn public_projection_entry_strips_owner_and_raw_gate_metadata() {
        let event = blocked_event(1, scope("thread-a"));
        let owner = event.owner_user_id.clone();
        assert!(owner.is_some());
        assert!(event.blocked_gate.is_some());

        let public = event.into_public_projection_entry();

        let serialized = serde_json::to_string(&public).expect("serialize public projection");
        assert!(!serialized.contains("gate:approval-a"));
        assert!(!serialized.contains("owner-a"));
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
            None,
            Some(EventCursor(5)),
            10,
            EventCursor(5),
        );

        assert_eq!(page.rebase_required, None);
        assert_eq!(page.entries, vec![event]);
        assert_eq!(page.next_cursor, EventCursor(6));
    }

    #[tokio::test]
    async fn projection_service_redacts_raw_gate_metadata_in_public_snapshot() {
        let scope = scope("thread-a");
        let source = MemoryProjectionSource {
            events: vec![blocked_event(1, scope.clone())],
        };
        let service = TurnEventProjectionService::new(std::sync::Arc::new(source));

        let snapshot = service
            .snapshot(crate::events::TurnEventProjectionRequest {
                scope,
                owner_user_id: None,
                after: None,
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(snapshot.entries.len(), 1);
        let serialized = serde_json::to_string(&snapshot).expect("serialize public snapshot");
        assert!(!serialized.contains("gate:approval-a"));
        assert!(!serialized.contains("owner-a"));
        assert_eq!(snapshot.entries[0].kind, TurnEventKind::Blocked);
    }

    #[tokio::test]
    async fn projection_service_preserves_source_read_error_cause() {
        use ironclaw_filesystem::{Fault, FaultInjecting, FilesystemOperation, InMemoryBackend};

        // The projection source is the real `FilesystemTurnStateRowStore` (which
        // implements `TurnEventProjectionSource`) over a `FaultInjecting` backend
        // armed to fail its first durable read. `read_turn_events_after` now runs
        // the store's genuine durable-row read and its
        // `FilesystemError::Backend -> TurnError::Unavailable` mapping, replacing
        // the former hand-rolled `FailingProjectionSource` fake. The service must
        // still wrap and preserve that read-error cause under the same
        // `read_turn_events_after` operation label.
        let backend = std::sync::Arc::new(FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::ReadFile).backend("injected turn-state read failure"),
        ));
        let source = std::sync::Arc::new(crate::FilesystemTurnStateRowStore::new(
            crate::test_support::scoped_turns_filesystem(backend),
        ));
        let service = TurnEventProjectionService::new(source);
        let error = service
            .snapshot(crate::events::TurnEventProjectionRequest {
                scope: scope("thread-source-error"),
                owner_user_id: None,
                after: None,
                limit: 10,
            })
            .await
            .expect_err("source error should propagate");

        // The cause is the real store's mapped error (`fs_error`), not the fake's
        // former "event store offline" string.
        assert!(matches!(
            error,
            TurnEventProjectionError::Source {
                operation: "read_turn_events_after",
                reason: TurnError::Unavailable { reason }
            } if reason == "turn state row-store persistence temporarily unavailable"
        ));
    }

    #[tokio::test]
    async fn reducer_snapshot_keeps_activity_id_while_public_snapshot_redacts_gate_metadata() {
        let scope = scope("thread-activity-id");
        let activity_id = CapabilityActivityId::new();
        let event = blocked_event_with_activity(1, scope.clone(), activity_id);
        let source = std::sync::Arc::new(MemoryProjectionSource {
            events: vec![event],
        });

        let reducer_snapshot = TurnEventReducerService::new(source.clone())
            .snapshot(crate::events::TurnEventProjectionRequest {
                scope: scope.clone(),
                owner_user_id: None,
                after: None,
                limit: 10,
            })
            .await
            .expect("reducer snapshot");

        assert_eq!(
            reducer_snapshot.entries[0]
                .blocked_gate
                .as_ref()
                .expect("reducer keeps gate metadata")
                .activity_id,
            Some(activity_id)
        );

        let public_snapshot = TurnEventProjectionService::new(source)
            .snapshot(crate::events::TurnEventProjectionRequest {
                scope,
                owner_user_id: None,
                after: None,
                limit: 10,
            })
            .await
            .expect("public snapshot");

        let serialized =
            serde_json::to_string(&public_snapshot).expect("serialize public snapshot");
        assert!(!serialized.contains("gate:approval-a"));
        assert!(!serialized.contains(&activity_id.to_string()));
    }

    #[tokio::test]
    async fn reducer_service_snapshot_preserves_raw_gate_metadata() {
        let scope = scope("thread-a");
        let source = MemoryProjectionSource {
            events: vec![blocked_event(1, scope.clone())],
        };
        let service = TurnEventReducerService::new(std::sync::Arc::new(source));

        let snapshot = service
            .snapshot(crate::events::TurnEventProjectionRequest {
                scope,
                owner_user_id: None,
                after: None,
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(snapshot.entries.len(), 1);
        let gate = snapshot.entries[0]
            .blocked_gate
            .as_ref()
            .expect("reducer service snapshot keeps gate metadata");
        assert_eq!(
            gate.gate_ref,
            GateRef::new("gate:approval-a").expect("gate ref")
        );
        assert_eq!(gate.gate_kind, TurnBlockedGateKind::Approval);
        assert_eq!(
            snapshot.entries[0].owner_user_id,
            Some(UserId::new("owner-a").expect("owner"))
        );
    }
}
