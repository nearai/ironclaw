use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_turns::{
    EventCursor as TurnEventCursor, GateRef, MAX_TURN_EVENT_PROJECTION_LIMIT, TurnBlockedGateKind,
    TurnEventKind, TurnEventProjectionSource, TurnEventSink, TurnLifecycleEvent, TurnRunId,
    TurnScope, TurnStatus,
};
use serde::{Deserialize, Serialize};

use crate::{MissingMetadataField, ProjectionError};

/// Stable consumer id used to persist pending-gate replay progress.
///
/// Cursor stores key this value with a [`TurnScope`] so replay progress for
/// this read model cannot collide with other turn-event consumers.
pub const PENDING_GATE_PROJECTION_CONSUMER_ID: &str = "pending_gate_projection.v1";

/// Pending gate category projected from a blocked turn lifecycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PendingGateKind {
    Approval,
    Auth,
    Resource,
    AwaitDependentRun,
}

impl From<TurnBlockedGateKind> for PendingGateKind {
    fn from(kind: TurnBlockedGateKind) -> Self {
        match kind {
            TurnBlockedGateKind::Approval => Self::Approval,
            TurnBlockedGateKind::Auth => Self::Auth,
            TurnBlockedGateKind::Resource => Self::Resource,
            TurnBlockedGateKind::AwaitDependentRun => Self::AwaitDependentRun,
        }
    }
}

/// Identity key for one projected pending gate row.
///
/// The key is scoped by tenant, optional agent/project, owner, thread, and run
/// so terminal/resume events can remove only the matching blocked run.
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

/// Materialized pending gate row produced by the projection consumer.
///
/// Rows intentionally carry only stable resolver metadata. They must not grow
/// to include approval reasons, raw prompts, tool input, backend details, or
/// host paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGateProjectionRow {
    pub key: PendingGateProjectionKey,
    /// Cursor of the lifecycle event that produced this row.
    ///
    /// Sinks use this as the per-key ordering guard so replay of an older
    /// blocked event cannot resurrect a gate that live delivery already
    /// removed with a newer terminal/resume event.
    pub source_cursor: TurnEventCursor,
    pub gate_kind: PendingGateKind,
    pub gate_ref: GateRef,
    pub blocked_at: Timestamp,
}

/// Storage boundary for projected pending gate rows.
///
/// Implementations must apply the cursor ordering guard described on each
/// method and must enforce bounded per-scope retention through row limits,
/// TTL eviction, or an equivalent storage policy — a turn flood that blocks
/// repeatedly on auth/approval gates can otherwise grow the read model
/// without bound. The projection layer is stateless and intentionally does
/// not duplicate this guard; the sink owns persistent state, so retention
/// belongs with whoever durably stores the row.
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

/// Durable cursor store for pending-gate replay progress.
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

/// Summary returned after replaying one scope page into the pending-gate sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingGateProjectionReplay {
    pub processed: usize,
    pub next_cursor: TurnEventCursor,
    pub truncated: bool,
}

/// Turn-event consumer that maintains the pending-gate read model.
///
/// Live [`TurnEventSink`] delivery updates rows without advancing the durable
/// replay cursor; explicit replay owns cursor progress so a crash cannot skip
/// turn lifecycle events that were not durably folded yet.
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

        if let Some(earliest) = page.rebase_required {
            return Err(ProjectionError::TurnEventRebaseRequired {
                requested: after,
                earliest,
            });
        }

        let mut processed = 0;
        let mut next_cursor = after;
        for event in page.entries {
            next_cursor = event.cursor;
            self.project_replay_event(event).await?;
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

    async fn project_event(&self, event: TurnLifecycleEvent) -> Result<(), ProjectionError> {
        match event.kind {
            TurnEventKind::Blocked => {
                // Defensive status guard for malformed legacy rows where
                // `kind == Blocked` but `status` is non-blocked. `gate_kind`
                // itself is read straight from `event.blocked_gate.gate_kind`
                // — no parallel derivation.
                if !matches!(
                    event.status,
                    TurnStatus::BlockedApproval
                        | TurnStatus::BlockedAuth
                        | TurnStatus::BlockedResource
                ) {
                    return Ok(());
                }
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
        Ok(())
    }

    async fn project_replay_event(&self, event: TurnLifecycleEvent) -> Result<(), ProjectionError> {
        // Keep only the kind (cheap to clone) for the error-tolerance check
        // so the happy path does not clone the full event.
        let kind = event.kind.clone();
        match self.project_event(event).await {
            Ok(()) => Ok(()),
            Err(ProjectionError::MissingProjectionMetadata { .. }) if is_replayable_kind(&kind) => {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Project an event and advance the durable replay cursor in one step.
    ///
    /// Test-only: production replay owns cursor progress in
    /// [`Self::replay_scope`] (batch advance per page) and live delivery does
    /// not advance the durable cursor. Exposing the combined step on the
    /// public surface would let a future caller double-advance the cursor.
    #[cfg(test)]
    pub(crate) async fn project_event_and_advance_cursor(
        &self,
        event: TurnLifecycleEvent,
    ) -> Result<(), ProjectionError> {
        let cursor = event.cursor;
        let scope = event.scope.clone();
        self.project_event(event).await?;
        self.cursor_store
            .advance_pending_gate_cursor(self.consumer_id, &scope, cursor)
            .await
    }
}

#[async_trait]
impl TurnEventSink for PendingGateProjection {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), ironclaw_turns::TurnError> {
        self.project_event(event)
            .await
            .map_err(|error| match error {
                ProjectionError::InvalidRequest { reason } => {
                    ironclaw_turns::TurnError::InvalidRequest {
                        reason: format!("pending gate projection failed: {reason}"),
                    }
                }
                ProjectionError::MissingProjectionMetadata { field } => {
                    ironclaw_turns::TurnError::InvalidRequest {
                        reason: format!(
                            "pending gate projection failed: {}",
                            field.as_static_str()
                        ),
                    }
                }
                // Avoid leaking the inner cursor values into the error reason
                // string crossing the turn-runtime boundary.
                ProjectionError::TurnEventRebaseRequired { .. } => {
                    ironclaw_turns::TurnError::Unavailable {
                        reason: "pending gate projection failed: turn event rebase required"
                            .to_string(),
                    }
                }
                ProjectionError::RebaseRequired { .. } => ironclaw_turns::TurnError::Unavailable {
                    reason: "pending gate projection failed: runtime rebase required".to_string(),
                },
                ProjectionError::Source { operation } => ironclaw_turns::TurnError::Unavailable {
                    reason: format!("pending gate projection failed: source {operation}"),
                },
            })
    }
}

fn row_from_blocked_event(
    event: &TurnLifecycleEvent,
) -> Result<PendingGateProjectionRow, ProjectionError> {
    let blocked_gate =
        event
            .blocked_gate
            .as_ref()
            .ok_or(ProjectionError::MissingProjectionMetadata {
                field: MissingMetadataField::BlockedGate,
            })?;
    let blocked_at = event
        .occurred_at
        .ok_or(ProjectionError::MissingProjectionMetadata {
            field: MissingMetadataField::OccurredAt,
        })?;

    Ok(PendingGateProjectionRow {
        key: key_from_lifecycle_event(event)?,
        source_cursor: event.cursor,
        gate_kind: blocked_gate.gate_kind.into(),
        gate_ref: blocked_gate.gate_ref.clone(),
        blocked_at,
    })
}

fn key_from_lifecycle_event(
    event: &TurnLifecycleEvent,
) -> Result<PendingGateProjectionKey, ProjectionError> {
    let owner_user_id =
        event
            .owner_user_id
            .clone()
            .ok_or(ProjectionError::MissingProjectionMetadata {
                field: MissingMetadataField::OwnerUserId,
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

/// `TurnLifecycleEvent` kinds that the pending-gate projection can derive a
/// row from (or remove a row for). Other kinds are silent-skip; the
/// replay-tolerance path uses this predicate to bound the kinds for which
/// missing legacy metadata may be ignored.
fn is_replayable_kind(kind: &TurnEventKind) -> bool {
    matches!(
        kind,
        TurnEventKind::Blocked
            | TurnEventKind::Completed
            | TurnEventKind::Failed
            | TurnEventKind::Cancelled
            | TurnEventKind::Resumed
    )
}

#[cfg(test)]
mod tests;
