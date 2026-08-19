//! Product-facing projections over Reborn durable runtime and audit logs.
//!
//! This crate is a read-model boundary. Upper Reborn layers should consume
//! these DTOs instead of parsing durable event/audit rows directly. The first
//! implementation is replay-derived over [`ironclaw_event_log::DurableEventLog`]
//! so it stays independent of concrete JSONL/PostgreSQL/libSQL adapters.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_log::{
    DurableAuditLog, DurableEventLog, EventError, EventLogEntry, EventStreamKey, ReadScope,
    RuntimeEvent, RuntimeEventKind, UNCLASSIFIED_ERROR_KIND, sanitize_error_kind,
    sanitize_recovery_label,
};
use ironclaw_host_api::{
    Timestamp,
    audit::{AuditEnvelope, AuditStage},
    decision::{OBLIGATION_EVALUATION_ORDER, ObligationKind},
    ids::{
        ApprovalRequestId, AuditEventId, CapabilityId, CorrelationId, ExtensionId, InvocationId,
        ProcessId, ThreadId,
    },
    resource::ResourceScope,
    runtime::RuntimeKind,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use ironclaw_event_log::EventCursor;

mod runtime_checkpoint_cache;
mod runtime_projection;
use runtime_checkpoint_cache::{RuntimeProjectionCheckpointCache, after_for_checkpoint};
use runtime_projection::{RuntimeProjectionState, capability_activity_transition_for_entry};

const STATE_REPLAY_PAGE_LIMIT: usize = 256;

/// Hard ceiling on how many runtime-prefix events `updates()` will fold while
/// reconstructing run state for touched invocations on a single call.
///
/// `updates()` does not collect the prefix into a `Vec`; it folds each page
/// incrementally so memory stays `O(touched_runs)` regardless of stream
/// length. This cap is a defense-in-depth against pathological streams (e.g.
/// a long-lived thread with millions of runtime events) where even paging
/// through the prefix would burn unbounded CPU on every poll. When the cap
/// is hit, the call surfaces [`ProjectionError::RebaseRequired`] so the
/// caller knows it must re-snapshot rather than silently see a partial
/// run-state view.
const STATE_REPLAY_MAX_EVENTS: usize = 100_000;

/// Maximum page size accepted by the projection service.
///
/// `ProjectionRequest.limit` is reserved for product adapters; a caller-
/// controlled limit must not be allowed to force the durable log to scan
/// or return an arbitrarily large page. Requests above this bound are
/// rejected with [`ProjectionError::InvalidRequest`] before any read.
pub const MAX_PROJECTION_PAGE_LIMIT: usize = 1_000;

/// Scoped projection request authority.
///
/// The stream key selects the durable `(tenant, user, agent)` partition. The
/// read scope tightens access within that partition so product callers cannot
/// observe neighboring project/thread/process records.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectionScope {
    pub stream: EventStreamKey,
    pub read_scope: ReadScope,
}

impl ProjectionScope {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            stream: EventStreamKey::from_scope(scope),
            read_scope: ReadScope {
                project_id: scope.project_id.clone(),
                mission_id: scope.mission_id.clone(),
                thread_id: scope.thread_id.clone(),
                process_id: None,
            },
        }
    }

    pub fn for_process(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            stream: EventStreamKey::from_scope(scope),
            read_scope: ReadScope {
                project_id: scope.project_id.clone(),
                mission_id: scope.mission_id.clone(),
                thread_id: scope.thread_id.clone(),
                process_id: Some(process_id),
            },
        }
    }
}

/// Cursor envelope for projection consumers.
///
/// This first slice is runtime-event backed. The wrapper keeps callers from
/// treating raw durable cursors as a stable product API and leaves room for
/// audit/materialized checkpoints later.
///
/// Cursors are **scope-bound**: every cursor carries the
/// [`ProjectionScope`] under which it was minted. The durable stream is
/// partitioned by `(tenant, user, agent)` while project / mission /
/// thread / process filtering happens inside the read filter, so a cursor
/// returned for thread B may have a runtime value that lies inside the
/// shared stream of thread A. Replaying it under thread A's scope without
/// scope-matching would silently skip thread A's earlier events. Resume
/// rejects mismatched-scope cursors with
/// [`ProjectionError::RebaseRequired`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProjectionCursor {
    pub runtime: EventCursor,
    pub scope: ProjectionScope,
}

impl ProjectionCursor {
    /// Construct a cursor bound to `scope` at the given runtime position.
    ///
    /// Production callers should let the service mint cursors via
    /// [`EventProjectionService::snapshot`] / [`EventProjectionService::updates`]
    /// and pass them straight back into the next request. Direct construction
    /// is provided for tests and adapters that already hold authority for
    /// the scope they pass in.
    pub fn for_scope(scope: ProjectionScope, runtime: EventCursor) -> Self {
        Self { runtime, scope }
    }

    /// Cursor that precedes every record in `scope`.
    pub fn origin_for_scope(scope: ProjectionScope) -> Self {
        Self {
            runtime: EventCursor::origin(),
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRequest {
    pub scope: ProjectionScope,
    pub after: Option<ProjectionCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSnapshot {
    pub timeline: ThreadTimeline,
    pub runs: Vec<RunStatusProjection>,
    pub capability_activities: Vec<CapabilityActivityProjection>,
    pub next_cursor: ProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionReplay {
    pub updates: Vec<TimelineEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_activity_transitions: Vec<CapabilityActivityProjection>,
    pub runs: Vec<RunStatusProjection>,
    pub capability_activities: Vec<CapabilityActivityProjection>,
    pub next_cursor: ProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadTimeline {
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub cursor: EventCursor,
    pub event_id: ironclaw_event_log::RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: TimelineEntryKind,
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    /// Sanitized hook metadata. Populated only when `kind` is one of the
    /// `Hook*` variants — for other kinds these fields are `None`.
    /// Each field is a *closed-vocabulary* label (no free-form text), so
    /// replay consumers can pattern-match on the actual hook that
    /// fired/failed without burning audit budget on operator-supplied
    /// reason strings (henrypark133 Concerning #6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_trust_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_failure_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_failure_disposition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_disposition: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEntryKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ModelStarted,
    ModelCompleted,
    ModelFailed,
    AssistantReplyFinalized,
    LoopCompleted,
    LoopCancelled,
    LoopFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
    HookDispatched,
    HookDecisionEmitted,
    HookFailed,
    FailureRecovered,
}

impl From<RuntimeEventKind> for TimelineEntryKind {
    fn from(kind: RuntimeEventKind) -> Self {
        match kind {
            RuntimeEventKind::DispatchRequested => Self::DispatchRequested,
            RuntimeEventKind::RuntimeSelected => Self::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded => Self::DispatchSucceeded,
            RuntimeEventKind::DispatchFailed => Self::DispatchFailed,
            RuntimeEventKind::CapabilityActivityRequested => Self::DispatchRequested,
            RuntimeEventKind::CapabilityActivitySucceeded => Self::DispatchSucceeded,
            RuntimeEventKind::CapabilityActivityFailed => Self::DispatchFailed,
            RuntimeEventKind::ModelStarted => Self::ModelStarted,
            RuntimeEventKind::ModelCompleted => Self::ModelCompleted,
            RuntimeEventKind::ModelFailed => Self::ModelFailed,
            RuntimeEventKind::AssistantReplyFinalized => Self::AssistantReplyFinalized,
            RuntimeEventKind::LoopCompleted => Self::LoopCompleted,
            RuntimeEventKind::LoopCancelled => Self::LoopCancelled,
            RuntimeEventKind::LoopFailed => Self::LoopFailed,
            RuntimeEventKind::ProcessStarted => Self::ProcessStarted,
            RuntimeEventKind::ProcessCompleted => Self::ProcessCompleted,
            RuntimeEventKind::ProcessFailed => Self::ProcessFailed,
            RuntimeEventKind::ProcessKilled => Self::ProcessKilled,
            RuntimeEventKind::HookDispatched => Self::HookDispatched,
            RuntimeEventKind::HookDecisionEmitted => Self::HookDecisionEmitted,
            RuntimeEventKind::HookFailed => Self::HookFailed,
            RuntimeEventKind::FailureRecovered => Self::FailureRecovered,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStatusProjection {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub thread_id: Option<ThreadId>,
    pub status: RunProjectionStatus,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub error_kind: Option<String>,
    pub last_cursor: EventCursor,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunProjectionStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityActivityProjection {
    pub invocation_id: InvocationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<InvocationId>,
    pub capability_id: CapabilityId,
    pub thread_id: Option<ThreadId>,
    pub status: CapabilityActivityStatus,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    /// Sanitized display detail derived from `RuntimeEvent.error_summary`.
    ///
    /// This intentionally uses the product-facing `error_detail` wire name:
    /// consumers render it as optional per-tool failure detail, not as the
    /// durable event's source summary field. Projection replay re-runs the
    /// runtime-event sanitizer before populating this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
    #[serde(default)]
    pub first_cursor: EventCursor,
    pub last_cursor: EventCursor,
    pub updated_at: Timestamp,
}

impl CapabilityActivityProjection {
    pub fn activity_order_cursor(&self) -> EventCursor {
        if self.first_cursor == EventCursor::origin() {
            self.last_cursor
        } else {
            self.first_cursor
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityActivityStatus {
    Started,
    Running,
    Completed,
    Failed,
    Killed,
}

/// Replay-tolerable projection metadata field absent from a legacy
/// `TurnLifecycleEvent`.
///
/// Variants are matched structurally by replay-tolerance logic; do not add a
/// `String` payload — match-on-string-literals is what this enum exists to
/// replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingMetadataField {
    /// Legacy blocked events may predate gate metadata; replay skips the
    /// derived pending-gate row rather than inventing a resolver reference.
    BlockedGate,
    /// Legacy blocked events may predate durable event timestamps; replay
    /// skips rows that cannot provide a stable blocked-at value.
    OccurredAt,
    /// Legacy turn events may predate owner metadata; replay skips rows whose
    /// pending-gate key cannot be scoped to the owning user.
    OwnerUserId,
}

impl MissingMetadataField {
    pub fn as_static_str(self) -> &'static str {
        match self {
            Self::BlockedGate => "blocked turn event missing gate metadata",
            Self::OccurredAt => "blocked turn event missing timestamp",
            Self::OwnerUserId => "turn event missing owner metadata",
        }
    }
}

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("projection request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    /// A `TurnLifecycleEvent` arrived without metadata the projection requires
    /// to materialize or key its row. Replay tolerates this for legacy events
    /// retained from before the metadata existed; live delivery surfaces it.
    #[error("projection metadata missing: {}", .field.as_static_str())]
    MissingProjectionMetadata { field: MissingMetadataField },
    #[error(
        "projection rebase required: requested runtime cursor {requested:?} cannot replay from earliest retained runtime cursor {earliest:?}"
    )]
    RebaseRequired {
        // Boxed because `ProjectionCursor` carries the full
        // `ProjectionScope` (stream + read scope) and inlining both
        // into the error variant balloons every `Result` size on the
        // happy path. Construction sites use `Box::new(..)`.
        requested: Box<ProjectionCursor>,
        earliest: Box<ProjectionCursor>,
    },
    #[error("projection source failed during {operation}")]
    Source { operation: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditProjectionCursor {
    pub audit: EventCursor,
    pub scope: ProjectionScope,
}

impl AuditProjectionCursor {
    pub fn for_scope(scope: ProjectionScope, audit: EventCursor) -> Self {
        Self { audit, scope }
    }

    pub fn origin_for_scope(scope: ProjectionScope) -> Self {
        Self {
            audit: EventCursor::origin(),
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionRequest {
    pub scope: ProjectionScope,
    pub after: Option<AuditProjectionCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionSnapshot {
    pub entries: Vec<AuditProjectionEntry>,
    pub next_cursor: AuditProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionReplay {
    pub entries: Vec<AuditProjectionEntry>,
    pub next_cursor: AuditProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionEntry {
    pub cursor: EventCursor,
    pub event_id: AuditEventId,
    pub timestamp: Timestamp,
    pub stage: AuditStage,
    pub correlation_id: CorrelationId,
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub process_id: Option<ProcessId>,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub extension_id: Option<ExtensionId>,
    pub action_kind: String,
    pub action_target: Option<String>,
    pub decision_kind: String,
    pub result_status: Option<String>,
    pub output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryAuditProjectionMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryAuditProjectionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_text: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_path_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_count: Option<u64>,
}

impl MemoryAuditProjectionMetadata {
    pub fn set_byte_count(mut self, byte_count: impl Into<Option<u64>>) -> Self {
        self.byte_count = byte_count.into();
        self
    }
}

#[derive(Debug, Error)]
pub enum AuditProjectionError {
    #[error("audit projection request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error(
        "audit projection rebase required: requested audit cursor {requested:?} cannot replay from earliest retained audit cursor {earliest:?}"
    )]
    RebaseRequired {
        requested: Box<AuditProjectionCursor>,
        earliest: Box<AuditProjectionCursor>,
    },
    #[error("audit projection source failed during {operation}")]
    Source { operation: &'static str },
}

#[async_trait]
pub trait AuditProjectionService: Send + Sync {
    async fn snapshot(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionSnapshot, AuditProjectionError>;

    async fn updates(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionReplay, AuditProjectionError>;
}

#[derive(Clone)]
pub struct ReplayAuditProjectionService {
    audit_log: Arc<dyn DurableAuditLog>,
}

impl ReplayAuditProjectionService {
    pub fn new<T>(audit_log: Arc<T>) -> Self
    where
        T: DurableAuditLog + 'static,
    {
        let audit_log: Arc<dyn DurableAuditLog> = audit_log;
        Self { audit_log }
    }

    pub fn from_audit_log(audit_log: Arc<dyn DurableAuditLog>) -> Self {
        Self { audit_log }
    }

    async fn read_audit(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<ProjectedAuditPage, AuditProjectionError> {
        if request.limit == 0 {
            return Err(AuditProjectionError::InvalidRequest {
                reason: "limit must be greater than zero",
            });
        }
        if request.limit > MAX_PROJECTION_PAGE_LIMIT {
            return Err(AuditProjectionError::InvalidRequest {
                reason: "limit exceeds MAX_PROJECTION_PAGE_LIMIT",
            });
        }
        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(AuditProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(AuditProjectionCursor::origin_for_scope(
                    request.scope.clone(),
                )),
            });
        }
        let fetch_limit =
            request
                .limit
                .checked_add(1)
                .ok_or(AuditProjectionError::InvalidRequest {
                    reason: "limit is too large",
                })?;
        let after = request.after.as_ref().map(|cursor| cursor.audit);
        let replay = self
            .audit_log
            .read_after_cursor(
                &request.scope.stream,
                &request.scope.read_scope,
                after,
                fetch_limit,
            )
            .await
            .map_err(|error| map_audit_projection_error(error, "audit replay", &request.scope))?;
        let mut entries = replay.entries;
        let truncated = entries.len() > request.limit;
        if truncated {
            entries.truncate(request.limit);
        }
        let next_cursor = if truncated {
            entries
                .last()
                .map(|entry| entry.cursor)
                .unwrap_or_else(|| after.unwrap_or_else(EventCursor::origin))
        } else {
            replay.next_cursor
        };
        Ok(ProjectedAuditPage {
            entries,
            next_cursor: AuditProjectionCursor::for_scope(request.scope.clone(), next_cursor),
            truncated,
        })
    }
}

impl std::fmt::Debug for ReplayAuditProjectionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplayAuditProjectionService")
            .field("audit_log", &"<durable_audit_log>")
            .finish()
    }
}

#[async_trait]
impl AuditProjectionService for ReplayAuditProjectionService {
    async fn snapshot(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionSnapshot, AuditProjectionError> {
        let page = self.read_audit(request).await?;
        Ok(AuditProjectionSnapshot {
            entries: project_audit_entries(&page.entries),
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }

    async fn updates(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionReplay, AuditProjectionError> {
        let page = self.read_audit(request).await?;
        Ok(AuditProjectionReplay {
            entries: project_audit_entries(&page.entries),
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }
}

#[async_trait]
pub trait EventProjectionService: Send + Sync {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError>;

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError>;
}

#[derive(Clone)]
pub struct ReplayEventProjectionService {
    runtime_log: Arc<dyn DurableEventLog>,
    runtime_checkpoints: RuntimeProjectionCheckpointCache,
}

impl ReplayEventProjectionService {
    pub fn new<T>(runtime_log: Arc<T>) -> Self
    where
        T: DurableEventLog + 'static,
    {
        let runtime_log: Arc<dyn DurableEventLog> = runtime_log;
        Self::from_runtime_log(runtime_log)
    }

    pub fn from_runtime_log(runtime_log: Arc<dyn DurableEventLog>) -> Self {
        Self {
            runtime_log,
            runtime_checkpoints: RuntimeProjectionCheckpointCache::default(),
        }
    }

    async fn read_runtime(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectedRuntimePage, ProjectionError> {
        if request.limit == 0 {
            return Err(ProjectionError::InvalidRequest {
                reason: "limit must be greater than zero",
            });
        }
        if request.limit > MAX_PROJECTION_PAGE_LIMIT {
            return Err(ProjectionError::InvalidRequest {
                reason: "limit exceeds MAX_PROJECTION_PAGE_LIMIT",
            });
        }
        // Reject cursors that were minted under a different scope. The
        // durable stream is partitioned by `(tenant, user, agent)`, so a
        // sibling thread/project/process within the same stream can mint
        // a runtime cursor that the durable log accepts but that would
        // silently skip records the requested scope had not yet seen.
        // Force the consumer to rebase against a snapshot instead of
        // returning a partial replay.
        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(ProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(ProjectionCursor::origin_for_scope(request.scope.clone())),
            });
        }
        let fetch_limit = request
            .limit
            .checked_add(1)
            .ok_or(ProjectionError::InvalidRequest {
                reason: "limit is too large",
            })?;
        let after = request.after.as_ref().map(|cursor| cursor.runtime);
        let replay = self
            .runtime_log
            .read_after_cursor(
                &request.scope.stream,
                &request.scope.read_scope,
                after,
                fetch_limit,
            )
            .await
            .map_err(|error| {
                map_projection_error(error, after, "runtime replay", &request.scope)
            })?;
        let mut entries = replay.entries;
        let truncated = entries.len() > request.limit;
        if truncated {
            entries.truncate(request.limit);
        }
        let next_cursor = if truncated {
            entries
                .last()
                .map(|entry| entry.cursor)
                .unwrap_or_else(|| after.unwrap_or_else(EventCursor::origin))
        } else {
            replay.next_cursor
        };
        Ok(ProjectedRuntimePage {
            entries,
            next_cursor: ProjectionCursor::for_scope(request.scope.clone(), next_cursor),
            truncated,
        })
    }

    /// Fold the entire scoped runtime stream into the current run-state
    /// projection for every invocation visible under `scope`.
    ///
    /// `snapshot()` uses this so the `runs` projection always reflects the
    /// current scoped stream head, independent of how the timeline page was
    /// paginated. Without this, a `snapshot(limit=1)` whose page contains
    /// only `DispatchRequested` for a run that has already terminated would
    /// surface a `Running` `RunStatusProjection` while the terminal event
    /// sits unread on the next page — silently shipping stale run state to
    /// consumers that use snapshots to rebase after a replay gap.
    ///
    /// The same bounded-memory contract applies: pages are folded
    /// incrementally, allocation is `O(scoped invocations)` regardless of
    /// stream length, and scanning more than [`STATE_REPLAY_MAX_EVENTS`]
    /// events surfaces [`ProjectionError::RebaseRequired`] instead of
    /// silently returning a partial run-state view. The requested limit is
    /// applied only to the emitted activity window after the fold, so late
    /// terminal events cannot be lost while compacting the output.
    async fn fold_runtime_to_head(
        &self,
        scope: &ProjectionScope,
        capability_activity_output_limit: usize,
    ) -> Result<RuntimeProjectionState, ProjectionError> {
        let mut checkpoint = self.runtime_checkpoints.latest(scope);
        let mut scanned: usize = 0;
        loop {
            let replay = self
                .runtime_log
                .read_after_cursor(
                    &scope.stream,
                    &scope.read_scope,
                    after_for_checkpoint(checkpoint.cursor),
                    STATE_REPLAY_PAGE_LIMIT,
                )
                .await
                .map_err(|error| {
                    map_projection_error(
                        error,
                        after_for_checkpoint(checkpoint.cursor),
                        "snapshot run-state replay",
                        scope,
                    )
                })?;
            if replay.entries.is_empty() {
                if replay.next_cursor > checkpoint.cursor {
                    checkpoint.cursor = replay.next_cursor;
                    self.runtime_checkpoints.store(scope, &checkpoint);
                    continue;
                }
                break;
            }
            for entry in &replay.entries {
                scanned = scanned.saturating_add(1);
                if scanned > STATE_REPLAY_MAX_EVENTS {
                    return Err(ProjectionError::RebaseRequired {
                        requested: Box::new(ProjectionCursor::origin_for_scope(scope.clone())),
                        earliest: Box::new(ProjectionCursor::for_scope(
                            scope.clone(),
                            entry.cursor,
                        )),
                    });
                }
                checkpoint.state.apply(entry);
            }
            if replay.next_cursor == checkpoint.cursor {
                // The durable log made no progress — stream exhausted.
                break;
            }
            checkpoint.cursor = replay.next_cursor;
            self.runtime_checkpoints.store(scope, &checkpoint);
        }
        Ok(checkpoint
            .state
            .with_output_limit(capability_activity_output_limit))
    }

    /// Fold the runtime-event prefix `(origin, until]` for `scope` into the
    /// run-state projection for the invocations identified by `touched`.
    ///
    /// This is the bounded-memory replacement for collecting the entire
    /// prefix into a `Vec`. The fold visits each page in sequence and only
    /// retains state for invocations the caller already saw in the current
    /// page, so allocation is `O(touched.len())` regardless of how many
    /// runtime events the stream has produced. A hard cap of
    /// [`STATE_REPLAY_MAX_EVENTS`] events scanned per call protects against
    /// pathological histories — when exceeded, the caller is told to rebase.
    async fn fold_runtime_prefix(
        &self,
        scope: &ProjectionScope,
        until: EventCursor,
        touched: &HashSet<InvocationId>,
    ) -> Result<RuntimeProjectionState, ProjectionError> {
        if touched.is_empty() || until == EventCursor::origin() {
            return Ok(RuntimeProjectionState::without_capability_activity_output_limit());
        }

        let mut checkpoint = self.runtime_checkpoints.at_or_before(scope, until);
        let mut scanned: usize = 0;
        loop {
            if checkpoint.cursor >= until {
                break;
            }
            let page_start_cursor = checkpoint.cursor;
            let replay = self
                .runtime_log
                .read_after_cursor(
                    &scope.stream,
                    &scope.read_scope,
                    after_for_checkpoint(checkpoint.cursor),
                    STATE_REPLAY_PAGE_LIMIT,
                )
                .await
                .map_err(|error| {
                    map_projection_error(
                        error,
                        after_for_checkpoint(checkpoint.cursor),
                        "runtime state replay",
                        scope,
                    )
                })?;
            if replay.entries.is_empty() {
                if replay.next_cursor > checkpoint.cursor {
                    checkpoint.cursor = replay.next_cursor.min(until);
                    self.runtime_checkpoints.store(scope, &checkpoint);
                    continue;
                }
                break;
            }

            for entry in &replay.entries {
                if entry.cursor > until {
                    checkpoint.state.retain_invocations(touched);
                    return Ok(checkpoint.state);
                }
                scanned = scanned.saturating_add(1);
                if scanned > STATE_REPLAY_MAX_EVENTS {
                    return Err(ProjectionError::RebaseRequired {
                        requested: Box::new(ProjectionCursor::for_scope(scope.clone(), until)),
                        earliest: Box::new(ProjectionCursor::for_scope(
                            scope.clone(),
                            entry.cursor,
                        )),
                    });
                }
                checkpoint.state.apply(entry);
                checkpoint.cursor = entry.cursor;
            }

            if replay.next_cursor > checkpoint.cursor {
                checkpoint.cursor = if replay.next_cursor > until {
                    until
                } else {
                    replay.next_cursor
                };
            }
            self.runtime_checkpoints.store(scope, &checkpoint);
            if checkpoint.cursor >= until || checkpoint.cursor == page_start_cursor {
                break;
            }
        }
        checkpoint.state.retain_invocations(touched);
        Ok(checkpoint.state)
    }
}

impl std::fmt::Debug for ReplayEventProjectionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplayEventProjectionService")
            .field("runtime_log", &"<durable_event_log>")
            .field("runtime_checkpoints", &"<runtime_projection_checkpoints>")
            .finish()
    }
}

#[async_trait]
impl EventProjectionService for ReplayEventProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        let scope = request.scope.clone();
        let limit = request.limit;
        let page = self.read_runtime(request).await?;
        let timeline = project_timeline(&page.entries);
        // Snapshot's `runs` always reflect the current scoped stream head,
        // not just the events present in `timeline`. A truncated timeline
        // page (or a `limit=1` request) would otherwise surface a stale
        // `Running` status for a run whose terminal event lives on the
        // next page — see PR #3212 review feedback (discussion_r3195454963).
        let folded = self.fold_runtime_to_head(&scope, limit).await?;
        let (runs, capability_activities) = folded.into_parts();
        Ok(ProjectionSnapshot {
            timeline,
            runs,
            capability_activities,
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        let scope = request.scope.clone();
        let page = self.read_runtime(request).await?;
        let capability_activity_transitions = page
            .entries
            .iter()
            .filter_map(capability_activity_transition_for_entry)
            .collect::<Vec<_>>();
        let touched_runs = page
            .entries
            .iter()
            .map(|entry| entry.record.scope.invocation_id)
            .collect::<HashSet<_>>();
        let (runs, capability_activities) = if touched_runs.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let folded = self
                .fold_runtime_prefix(&scope, page.next_cursor.runtime, &touched_runs)
                .await?;
            folded.into_parts()
        };
        Ok(ProjectionReplay {
            updates: project_timeline(&page.entries).entries,
            capability_activity_transitions,
            runs,
            capability_activities,
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }
}

struct ProjectedAuditPage {
    entries: Vec<EventLogEntry<AuditEnvelope>>,
    next_cursor: AuditProjectionCursor,
    truncated: bool,
}

fn project_audit_entries(entries: &[EventLogEntry<AuditEnvelope>]) -> Vec<AuditProjectionEntry> {
    entries.iter().map(project_audit_entry).collect()
}

fn project_audit_entry(entry: &EventLogEntry<AuditEnvelope>) -> AuditProjectionEntry {
    let audit = &entry.record;
    let action_kind = sanitize_error_kind(audit.action.kind.clone());
    let output_bytes = audit.result.as_ref().and_then(|result| result.output_bytes);
    let memory = audit
        .result
        .as_ref()
        .and_then(|result| result.status.as_deref())
        .and_then(|status| parse_memory_audit_metadata(status, output_bytes));
    let result_status = if let Some(memory) = &memory {
        memory.status.clone()
    } else {
        audit
            .result
            .as_ref()
            .and_then(|result| result.status.as_deref())
            .map(sanitize_audit_status)
    };
    AuditProjectionEntry {
        cursor: entry.cursor,
        event_id: audit.event_id,
        timestamp: audit.timestamp,
        stage: audit.stage,
        correlation_id: audit.correlation_id,
        invocation_id: audit.invocation_id,
        thread_id: audit.thread_id.clone(),
        process_id: audit.process_id,
        approval_request_id: audit.approval_request_id,
        extension_id: audit.extension_id.clone(),
        action_target: safe_audit_action_target(&action_kind, audit.action.target.as_ref()),
        action_kind,
        decision_kind: sanitize_error_kind(audit.decision.kind.clone()),
        result_status,
        output_bytes,
        memory,
    }
}

fn parse_memory_audit_metadata(
    status: &str,
    output_bytes: Option<u64>,
) -> Option<MemoryAuditProjectionMetadata> {
    let mut segments = status.split(';');
    let prefix = segments.next()?;
    if prefix != "memory_event:v1" && prefix != "memory_prompt_safety:v1" {
        return None;
    }

    let mut metadata = MemoryAuditProjectionMetadata::default().set_byte_count(output_bytes);
    for segment in segments {
        let (key, value) = segment.split_once('=')?;
        match key {
            "status" => metadata.status = sanitize_memory_metadata_label(value),
            "path_hash" => metadata.relative_path_hash = sanitize_memory_path_hash(value),
            "chunks" => metadata.chunk_count = sanitize_memory_metadata_u64(value),
            "results" => metadata.result_count = sanitize_memory_metadata_u64(value),
            "full_text" => metadata.full_text = sanitize_memory_metadata_bool(value),
            "vector" => metadata.vector = sanitize_memory_metadata_bool(value),
            "protected_path_class" => {
                metadata.protected_path_class = sanitize_memory_metadata_label(value)
            }
            "reason" => metadata.reason_code = sanitize_memory_metadata_label(value),
            "severity" => metadata.severity = sanitize_memory_metadata_label(value),
            "findings" => metadata.finding_count = sanitize_memory_metadata_u64(value),
            _ => return None,
        }
    }
    metadata.status.as_ref()?;
    Some(metadata)
}

fn sanitize_memory_metadata_label(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 128 {
        return None;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        .then(|| value.to_string())
}

fn sanitize_memory_path_hash(value: &str) -> Option<String> {
    let hash = value.strip_prefix("sha256:").unwrap_or(value);
    (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn sanitize_memory_metadata_u64(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn sanitize_memory_metadata_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn sanitize_audit_status(status: &str) -> String {
    let mut seen = HashSet::new();
    let mut sanitized = String::new();

    for (index, label) in status.split(',').enumerate() {
        if index >= OBLIGATION_EVALUATION_ORDER.len() {
            return UNCLASSIFIED_ERROR_KIND.to_string();
        }

        let Some(kind) = obligation_kind_from_status_label(label) else {
            return UNCLASSIFIED_ERROR_KIND.to_string();
        };
        if !seen.insert(kind) {
            return UNCLASSIFIED_ERROR_KIND.to_string();
        }

        if index > 0 {
            sanitized.push(',');
        }
        sanitized.push_str(label);
    }

    if seen.is_empty() {
        UNCLASSIFIED_ERROR_KIND.to_string()
    } else {
        sanitized
    }
}

fn obligation_kind_from_status_label(label: &str) -> Option<ObligationKind> {
    OBLIGATION_EVALUATION_ORDER
        .iter()
        .copied()
        .find(|kind| obligation_status_label(*kind) == label)
}

fn obligation_status_label(kind: ObligationKind) -> &'static str {
    match kind {
        ObligationKind::ReserveResources => "reserve_resources",
        ObligationKind::UseScopedMounts => "use_scoped_mounts",
        ObligationKind::ApplyNetworkPolicy => "apply_network_policy",
        ObligationKind::InjectSecretOnce => "inject_secret_once",
        ObligationKind::InjectCredentialAccountOnce => "inject_credential_account_once",
        ObligationKind::FirstPartyCredentialStagedViaHostPort => {
            "first_party_credential_staged_via_host_port"
        }
        ObligationKind::AuditBefore => "audit_before",
        ObligationKind::RedactOutput => "redact_output",
        ObligationKind::EnforceResourceCeiling => "enforce_resource_ceiling",
        ObligationKind::EnforceOutputLimit => "enforce_output_limit",
        ObligationKind::AuditAfter => "audit_after",
    }
}

fn safe_audit_action_target(action_kind: &str, target: Option<&String>) -> Option<String> {
    match action_kind {
        "dispatch" | "spawn_capability" => target.and_then(|target| {
            CapabilityId::new(target.clone())
                .ok()
                .map(|capability| capability.into_string())
        }),
        _ => None,
    }
}

struct ProjectedRuntimePage {
    entries: Vec<EventLogEntry<RuntimeEvent>>,
    next_cursor: ProjectionCursor,
    truncated: bool,
}

fn project_timeline(entries: &[EventLogEntry<RuntimeEvent>]) -> ThreadTimeline {
    ThreadTimeline {
        entries: entries.iter().map(project_timeline_entry).collect(),
    }
}

fn project_timeline_entry(entry: &EventLogEntry<RuntimeEvent>) -> TimelineEntry {
    let event = &entry.record;
    TimelineEntry {
        cursor: entry.cursor,
        event_id: event.event_id,
        timestamp: event.timestamp,
        kind: event.kind.into(),
        invocation_id: event.scope.invocation_id,
        thread_id: event.scope.thread_id.clone(),
        capability_id: event.capability_id.clone(),
        provider: event.provider.clone(),
        runtime: event.runtime,
        process_id: event.process_id,
        output_bytes: event.output_bytes,
        error_kind: event.error_kind.clone().map(sanitize_error_kind),
        hook_id: event.hook_id.clone(),
        hook_point: event.hook_point.clone(),
        hook_trust_class: event.hook_trust_class.clone(),
        hook_decision: event.hook_decision.clone(),
        hook_failure_category: event.hook_failure_category.clone(),
        hook_failure_disposition: event.hook_failure_disposition.clone(),
        recovery_stage: event.recovery_stage.clone().map(sanitize_recovery_label),
        recovery_class: event.recovery_class.clone().map(sanitize_recovery_label),
        recovery_disposition: event
            .recovery_disposition
            .clone()
            .map(sanitize_recovery_label),
    }
}

fn map_audit_projection_error(
    error: EventError,
    operation: &'static str,
    scope: &ProjectionScope,
) -> AuditProjectionError {
    match error {
        EventError::ReplayGap {
            requested,
            earliest,
        } => AuditProjectionError::RebaseRequired {
            requested: Box::new(AuditProjectionCursor::for_scope(scope.clone(), requested)),
            earliest: Box::new(AuditProjectionCursor::for_scope(scope.clone(), earliest)),
        },
        EventError::InvalidReplayRequest { .. } => AuditProjectionError::InvalidRequest {
            reason: "invalid durable replay request",
        },
        EventError::Serialize { .. } | EventError::Sink { .. } | EventError::DurableLog { .. } => {
            AuditProjectionError::Source { operation }
        }
    }
}

fn map_projection_error(
    error: EventError,
    _requested_after: Option<EventCursor>,
    operation: &'static str,
    scope: &ProjectionScope,
) -> ProjectionError {
    match error {
        EventError::ReplayGap {
            requested,
            earliest,
        } => ProjectionError::RebaseRequired {
            requested: Box::new(ProjectionCursor::for_scope(scope.clone(), requested)),
            earliest: Box::new(ProjectionCursor::for_scope(scope.clone(), earliest)),
        },
        EventError::InvalidReplayRequest { .. } => ProjectionError::InvalidRequest {
            reason: "invalid durable replay request",
        },
        EventError::Serialize { .. } | EventError::Sink { .. } | EventError::DurableLog { .. } => {
            ProjectionError::Source { operation }
        }
    }
}
