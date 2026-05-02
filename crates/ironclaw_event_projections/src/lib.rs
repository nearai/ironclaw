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
    DurableEventLog, EventCursor, EventError, EventLogEntry, EventStreamKey, ReadScope,
    RuntimeEvent, RuntimeEventKind,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ProcessId, ResourceScope, RuntimeKind, ThreadId,
    Timestamp,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const STATE_REPLAY_PAGE_LIMIT: usize = 256;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProjectionCursor {
    pub runtime: EventCursor,
}

impl ProjectionCursor {
    pub fn new(runtime: EventCursor) -> Self {
        Self { runtime }
    }

    pub fn origin() -> Self {
        Self {
            runtime: EventCursor::origin(),
        }
    }
}

impl Default for ProjectionCursor {
    fn default() -> Self {
        Self::origin()
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
        requested: ProjectionCursor,
        earliest: ProjectionCursor,
    },
    #[error("projection source failed during {operation}")]
    Source { operation: &'static str },
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
        let fetch_limit = request
            .limit
            .checked_add(1)
            .ok_or(ProjectionError::InvalidRequest {
                reason: "limit is too large",
            })?;
        let after = request.after.map(|cursor| cursor.runtime);
        let replay = self
            .runtime_log
            .read_after_cursor(
                &request.scope.stream,
                &request.scope.read_scope,
                after,
                fetch_limit,
            )
            .await
            .map_err(|error| map_projection_error(error, after, "runtime replay"))?;
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
            next_cursor: ProjectionCursor::new(next_cursor),
            truncated,
        })
    }

    async fn read_runtime_prefix(
        &self,
        scope: &ProjectionScope,
        until: EventCursor,
    ) -> Result<Vec<EventLogEntry<RuntimeEvent>>, ProjectionError> {
        if until == EventCursor::origin() {
            return Ok(Vec::new());
        }

        let mut after = None;
        let mut entries = Vec::new();
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
                .map_err(|error| map_projection_error(error, after, "runtime state replay"))?;
            if replay.entries.is_empty() {
                break;
            }

            for entry in replay.entries {
                if entry.cursor > until {
                    return Ok(entries);
                }
                let cursor = entry.cursor;
                entries.push(entry);
                if cursor >= until {
                    return Ok(entries);
                }
            }

            if replay.next_cursor >= until || after == Some(replay.next_cursor) {
                break;
            }
            after = Some(replay.next_cursor);
        }
        Ok(entries)
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
        let page = self.read_runtime(request).await?;
        let timeline = project_timeline(&page.entries);
        let runs = project_runs(&page.entries);
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
            let prefix = self
                .read_runtime_prefix(&scope, page.next_cursor.runtime)
                .await?;
            let mut runs = project_runs(&prefix);
            runs.retain(|run| touched_runs.contains(&run.invocation_id));
            runs
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
        error_kind: event.error_kind.clone(),
    }
}

fn project_runs(entries: &[EventLogEntry<RuntimeEvent>]) -> Vec<RunStatusProjection> {
    let mut runs = HashMap::<InvocationId, RunStatusProjection>::new();
    for entry in entries {
        apply_run_event(&mut runs, entry);
    }
    let mut runs = runs.into_values().collect::<Vec<_>>();
    sort_runs_for_projection(&mut runs);
    runs
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
            error_kind: event.error_kind.clone(),
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
    if event.error_kind.is_some() {
        run.error_kind = event.error_kind.clone();
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
        RuntimeEventKind::DispatchSucceeded | RuntimeEventKind::ProcessCompleted => {
            RunProjectionStatus::Completed
        }
        RuntimeEventKind::DispatchFailed | RuntimeEventKind::ProcessFailed => {
            RunProjectionStatus::Failed
        }
        RuntimeEventKind::ProcessKilled => RunProjectionStatus::Killed,
    }
}

fn map_projection_error(
    error: EventError,
    _requested_after: Option<EventCursor>,
    operation: &'static str,
) -> ProjectionError {
    match error {
        EventError::ReplayGap {
            requested,
            earliest,
        } => ProjectionError::RebaseRequired {
            requested: ProjectionCursor::new(requested),
            earliest: ProjectionCursor::new(earliest),
        },
        EventError::InvalidReplayRequest { .. } => ProjectionError::InvalidRequest {
            reason: "invalid durable replay request",
        },
        EventError::Serialize { .. } | EventError::Sink { .. } | EventError::DurableLog { .. } => {
            ProjectionError::Source { operation }
        }
    }
}
