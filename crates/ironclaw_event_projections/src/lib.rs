//! Product-facing projections over Reborn durable runtime and audit logs.
//!
//! This crate is a read-model boundary. Upper Reborn layers should consume
//! these DTOs instead of parsing durable event/audit rows directly. The first
//! implementation is replay-derived over [`ironclaw_events::DurableEventLog`]
//! so it stays independent of concrete JSONL/PostgreSQL/libSQL adapters.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry, EventStreamKey,
    ReadScope, RuntimeEvent, RuntimeEventKind, sanitize_error_kind,
};
use ironclaw_host_api::{
    ApprovalRequestId, AuditEnvelope, AuditEventId, AuditStage, CapabilityId, ExtensionId,
    InvocationId, ProcessId, ResourceScope, RuntimeKind, ThreadId, Timestamp,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub next_cursor: ProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionReplay {
    pub updates: Vec<TimelineEntry>,
    pub runs: Vec<RunStatusProjection>,
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
    pub event_id: ironclaw_events::RuntimeEventId,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEntryKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
}

impl From<RuntimeEventKind> for TimelineEntryKind {
    fn from(kind: RuntimeEventKind) -> Self {
        match kind {
            RuntimeEventKind::DispatchRequested => Self::DispatchRequested,
            RuntimeEventKind::RuntimeSelected => Self::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded => Self::DispatchSucceeded,
            RuntimeEventKind::DispatchFailed => Self::DispatchFailed,
            RuntimeEventKind::ProcessStarted => Self::ProcessStarted,
            RuntimeEventKind::ProcessCompleted => Self::ProcessCompleted,
            RuntimeEventKind::ProcessFailed => Self::ProcessFailed,
            RuntimeEventKind::ProcessKilled => Self::ProcessKilled,
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
    Failed,
    Killed,
}

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("projection request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
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
    pub stage: AuditProjectionStage,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditProjectionStage {
    Before,
    After,
    Denied,
    ApprovalRequested,
    ApprovalResolved,
    ResourceReserved,
    ResourceReconciled,
    ResourceReleased,
}

impl From<AuditStage> for AuditProjectionStage {
    fn from(stage: AuditStage) -> Self {
        match stage {
            AuditStage::Before => Self::Before,
            AuditStage::After => Self::After,
            AuditStage::Denied => Self::Denied,
            AuditStage::ApprovalRequested => Self::ApprovalRequested,
            AuditStage::ApprovalResolved => Self::ApprovalResolved,
            AuditStage::ResourceReserved => Self::ResourceReserved,
            AuditStage::ResourceReconciled => Self::ResourceReconciled,
            AuditStage::ResourceReleased => Self::ResourceReleased,
        }
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
}

impl ReplayEventProjectionService {
    pub fn new<T>(runtime_log: Arc<T>) -> Self
    where
        T: DurableEventLog + 'static,
    {
        let runtime_log: Arc<dyn DurableEventLog> = runtime_log;
        Self { runtime_log }
    }

    pub fn from_runtime_log(runtime_log: Arc<dyn DurableEventLog>) -> Self {
        Self { runtime_log }
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
    /// incrementally, allocation is `O(scoped runs)` regardless of stream
    /// length, and scanning more than [`STATE_REPLAY_MAX_EVENTS`] events
    /// surfaces [`ProjectionError::RebaseRequired`] instead of silently
    /// returning a partial run-state view.
    async fn fold_runtime_to_head(
        &self,
        scope: &ProjectionScope,
    ) -> Result<HashMap<InvocationId, RunStatusProjection>, ProjectionError> {
        let mut runs = HashMap::<InvocationId, RunStatusProjection>::new();
        let mut after: Option<EventCursor> = None;
        let mut scanned: usize = 0;
        loop {
            let replay = self
                .runtime_log
                .read_after_cursor(
                    &scope.stream,
                    &scope.read_scope,
                    after,
                    STATE_REPLAY_PAGE_LIMIT,
                )
                .await
                .map_err(|error| {
                    map_projection_error(error, after, "snapshot run-state replay", scope)
                })?;
            if replay.entries.is_empty() {
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
                apply_run_event(&mut runs, entry);
            }
            if after == Some(replay.next_cursor) {
                // The durable log made no progress — stream exhausted.
                break;
            }
            after = Some(replay.next_cursor);
        }
        Ok(runs)
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
    ) -> Result<HashMap<InvocationId, RunStatusProjection>, ProjectionError> {
        let mut runs = HashMap::<InvocationId, RunStatusProjection>::new();
        if touched.is_empty() || until == EventCursor::origin() {
            return Ok(runs);
        }

        let mut after = None;
        let mut scanned: usize = 0;
        loop {
            let replay = self
                .runtime_log
                .read_after_cursor(
                    &scope.stream,
                    &scope.read_scope,
                    after,
                    STATE_REPLAY_PAGE_LIMIT,
                )
                .await
                .map_err(|error| {
                    map_projection_error(error, after, "runtime state replay", scope)
                })?;
            if replay.entries.is_empty() {
                break;
            }

            for entry in &replay.entries {
                if entry.cursor > until {
                    return Ok(runs);
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
                if touched.contains(&entry.record.scope.invocation_id) {
                    apply_run_event(&mut runs, entry);
                }
                if entry.cursor >= until {
                    return Ok(runs);
                }
            }

            if replay.next_cursor >= until || after == Some(replay.next_cursor) {
                break;
            }
            after = Some(replay.next_cursor);
        }
        Ok(runs)
    }
}

impl std::fmt::Debug for ReplayEventProjectionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplayEventProjectionService")
            .field("runtime_log", &"<durable_event_log>")
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
        let page = self.read_runtime(request).await?;
        let timeline = project_timeline(&page.entries);
        // Snapshot's `runs` always reflect the current scoped stream head,
        // not just the events present in `timeline`. A truncated timeline
        // page (or a `limit=1` request) would otherwise surface a stale
        // `Running` status for a run whose terminal event lives on the
        // next page — see PR #3212 review feedback (discussion_r3195454963).
        let folded = self.fold_runtime_to_head(&scope).await?;
        let mut runs: Vec<RunStatusProjection> = folded.into_values().collect();
        sort_runs_for_projection(&mut runs);
        Ok(ProjectionSnapshot {
            timeline,
            runs,
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
        let touched_runs = page
            .entries
            .iter()
            .map(|entry| entry.record.scope.invocation_id)
            .collect::<HashSet<_>>();
        let mut runs = if touched_runs.is_empty() {
            Vec::new()
        } else {
            let folded = self
                .fold_runtime_prefix(&scope, page.next_cursor.runtime, &touched_runs)
                .await?;
            folded.into_values().collect::<Vec<_>>()
        };
        sort_runs_for_projection(&mut runs);
        Ok(ProjectionReplay {
            updates: project_timeline(&page.entries).entries,
            runs,
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
    AuditProjectionEntry {
        cursor: entry.cursor,
        event_id: audit.event_id,
        timestamp: audit.timestamp,
        stage: audit.stage.into(),
        invocation_id: audit.invocation_id,
        thread_id: audit.thread_id.clone(),
        process_id: audit.process_id,
        approval_request_id: audit.approval_request_id,
        extension_id: audit.extension_id.clone(),
        action_target: safe_audit_action_target(&action_kind, audit.action.target.as_ref()),
        action_kind,
        decision_kind: sanitize_error_kind(audit.decision.kind.clone()),
        result_status: audit
            .result
            .as_ref()
            .and_then(|result| result.status.clone())
            .map(sanitize_error_kind),
        output_bytes: audit.result.as_ref().and_then(|result| result.output_bytes),
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
    }
}

fn sort_runs_for_projection(runs: &mut [RunStatusProjection]) {
    runs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.last_cursor.cmp(&left.last_cursor))
            .then_with(|| {
                left.invocation_id
                    .as_uuid()
                    .cmp(&right.invocation_id.as_uuid())
            })
    });
}

fn apply_run_event(
    runs: &mut HashMap<InvocationId, RunStatusProjection>,
    entry: &EventLogEntry<RuntimeEvent>,
) {
    let event = &entry.record;
    let existing = runs.get(&event.scope.invocation_id);
    let status = run_status_for_event(
        event.kind,
        existing.map(|run| run.status),
        existing.and_then(|run| run.process_id).is_some(),
    );
    let sanitized_error_kind = event.error_kind.clone().map(sanitize_error_kind);
    let run = runs
        .entry(event.scope.invocation_id)
        .or_insert_with(|| RunStatusProjection {
            invocation_id: event.scope.invocation_id,
            capability_id: event.capability_id.clone(),
            thread_id: event.scope.thread_id.clone(),
            status,
            provider: event.provider.clone(),
            runtime: event.runtime,
            process_id: event.process_id,
            error_kind: sanitized_error_kind.clone(),
            last_cursor: entry.cursor,
            updated_at: event.timestamp,
        });

    run.status = status;
    run.capability_id = event.capability_id.clone();
    run.thread_id = event.scope.thread_id.clone();
    if event.provider.is_some() {
        run.provider = event.provider.clone();
    }
    if event.runtime.is_some() {
        run.runtime = event.runtime;
    }
    if event.process_id.is_some() {
        run.process_id = event.process_id;
    }
    if sanitized_error_kind.is_some() {
        run.error_kind = sanitized_error_kind;
    }
    run.last_cursor = entry.cursor;
    run.updated_at = event.timestamp;
}

fn run_status_for_event(
    kind: RuntimeEventKind,
    current_status: Option<RunProjectionStatus>,
    has_active_process: bool,
) -> RunProjectionStatus {
    match kind {
        RuntimeEventKind::DispatchRequested
        | RuntimeEventKind::RuntimeSelected
        | RuntimeEventKind::ProcessStarted => RunProjectionStatus::Running,
        RuntimeEventKind::DispatchSucceeded
            if has_active_process && current_status == Some(RunProjectionStatus::Running) =>
        {
            RunProjectionStatus::Running
        }
        // For process-backed runs, `DispatchSucceeded` may simply acknowledge
        // that a background process was spawned. If the process trail has
        // already terminated (`Failed` or `Killed`), a late `DispatchSucceeded`
        // must NOT overwrite that terminal status — doing so would silently
        // hide failures from product consumers.
        RuntimeEventKind::DispatchSucceeded
            if has_active_process
                && matches!(
                    current_status,
                    Some(RunProjectionStatus::Failed) | Some(RunProjectionStatus::Killed)
                ) =>
        {
            current_status.unwrap_or(RunProjectionStatus::Failed)
        }
        RuntimeEventKind::DispatchSucceeded | RuntimeEventKind::ProcessCompleted => {
            RunProjectionStatus::Completed
        }
        RuntimeEventKind::DispatchFailed | RuntimeEventKind::ProcessFailed => {
            RunProjectionStatus::Failed
        }
        RuntimeEventKind::ProcessKilled => RunProjectionStatus::Killed,
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
